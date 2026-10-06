use super::startup::Client;
use super::*;

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    assert!(
        fixture
            .run(&["new-instance", "review", "-t", "website"])
            .status
            .success()
    );
    fs::create_dir(fixture.home.join("templates/blank")).unwrap();
    fs::write(fixture.home.join("templates/blank/tandem.json"), "{}").unwrap();
    assert!(
        fixture
            .run(&["new-instance", "reader", "-t", "blank"])
            .status
            .success()
    );
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    connection.busy_timeout(Duration::from_secs(5)).unwrap();
    connection
        .execute(
            "INSERT INTO event_providers(namespace, name, token) VALUES ('cli-test', 'sample', ?1)",
            ["a".repeat(64)],
        )
        .unwrap();
    let event = json!({"schema_version": 1, "event_id": "one", "stream": "fixture", "profile": "message", "type": "fixture.message", "summary": "Inspect the incident", "data": {"author": "Test", "channel": "Fixture", "text": "Inspect"}});
    connection.execute("INSERT INTO events(namespace, provider, event_id, payload, received_at) VALUES ('cli-test', 'sample', 'one', ?1, '2026-01-01T00:00:00Z')", [event.to_string()]).unwrap();
    connection.execute("INSERT INTO event_attempts(event_sequence, status, replay, created_at) VALUES (1, 'accepted', 0, '2026-01-01T00:00:00Z')", []).unwrap();
    let rule = json!({"definition": {
        "name": "inspect", "description": "Fixture", "script": "fn matches(event) { true }",
        "template": "website", "model": "openai/test", "initial_prompt": "Inspect", "enabled": true,
        "start_instance": true
    }, "revision": 1, "zellij_session": "main"});
    let acceptance = json!({
        "id": 1, "event_sequence": 1, "event_summary": "Inspect the incident", "attempt_id": 1,
        "rule_name": "inspect", "rule_revision": 1, "accepted_at": "2026-01-01T00:00:00Z",
        "instance": "review", "session_id": "ses_incident", "pane": null,
        "operation_id": fixture.runtime_record("review", "startup").unwrap()["operation"]["id"],
        "launch_started_at": "2026-01-01T00:00:00Z", "status": "launched", "error": null,
        "rule": rule, "resolved_prompt": "Inspect"
    });
    connection.execute("INSERT INTO rule_acceptances(attempt_id, rule_name, payload) VALUES (1, 'inspect', ?1)", [acceptance.to_string()]).unwrap();
    // Conclusion closes associated clients even with observation disabled.
    connection.execute("INSERT INTO app_settings(key, value) VALUES ('integrations.opencode', 'false') ON CONFLICT(key) DO UPDATE SET value='false'", []).unwrap();
    fs::rename(fixture.bin.join("zellij"), fixture.bin.join("zellij-base")).unwrap();
    fs::write(fixture.bin.join("zellij"), ZELLIJ).unwrap();
    fs::set_permissions(
        fixture.bin.join("zellij"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let presence = fixture.home.join("state/tandem/opencode");
    fs::create_dir_all(&presence).unwrap();
    fs::write(presence.join("incident.json"), json!({
        "pid": std::process::id(), "observed_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
        "id": "ses_incident", "title": "Incident", "directory": fixture.home.join("workspaces/review"),
        "server": "", "activity": "idle", "zellij_session": "main", "pane_id": 100
    }).to_string()).unwrap();
    fixture
}

fn client(fixture: &Fixture, name: &str) -> Client {
    let mut command = fixture.command(&["mcp-instance"]);
    command
        .current_dir(fixture.home.join("workspaces").join(name))
        .stderr(Stdio::inherit());
    Client::with_command(command)
}

fn input() -> serde_json::Value {
    json!({"title": "Incident resolved", "summary": "Verified service recovery", "markdown": "# Investigation\n\nThe hidden timeout was reproduced; verification passed.\n"})
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "conclusion condition timed out");
        thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn scoped_conclusion_survives_mcp_kill_and_retains_searchable_reports_after_resource_purge() {
    let fixture = fixture();
    let mut agent = client(&fixture, "review");
    let catalog = agent.request("tools/list", json!({}));
    let mut names: Vec<_> = catalog["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "conclude",
            "get_event_report",
            "get_instructions",
            "search_events",
            "start_self",
            "stop_self",
            "update_repositories"
        ]
    );
    let injection = agent.request_raw("tools/call", json!({"name": "conclude", "arguments": {"title": "Other", "summary": "Other", "markdown": "Other", "name": "reader"}}));
    assert_eq!(injection["error"]["code"], -32602);
    let receipt = agent.tool("conclude", input());
    assert_eq!(receipt["acceptance_id"], 1);
    assert_eq!(receipt["cleanup_state"], "pending");
    wait_for(|| fixture.home.join("closing").exists());
    let mut reader = client(&fixture, "reader");
    assert_eq!(
        reader.tool("get_event_report", json!({"acceptance_id": 1}))["markdown"],
        input()["markdown"]
    );
    assert!(fixture.home.join("workspaces/review").is_dir());
    agent.close(Some(libc::SIGKILL));
    fs::write(fixture.home.join("release-close"), "").unwrap();
    wait_for(|| {
        reader.tool("get_event_report", json!({"acceptance_id": 1}))["cleanup_state"] == "purged"
    });
    assert!(!fixture.home.join("workspaces/review").exists());
    assert!(fixture.home.join("workspaces/reader").is_dir());
    assert!(fixture.runtime_record("review", "journal").is_none());
    assert!(fixture.runtime_record("review", "startup").is_none());
    let results = reader.tool(
        "search_events",
        json!({"search_strings": ["not matched", "HIDDEN TIMEOUT", "incident"]}),
    )["reports"]
        .clone();
    assert_eq!(results.as_array().unwrap().len(), 1);
    assert_eq!(results[0]["title"], "Incident resolved");
    assert_eq!(results[0]["summary"], "Verified service recovery");
    let mut management = Client::with_command(fixture.command(&["mcp"]));
    assert_eq!(
        management.tool("get_event_report", json!({"acceptance_id": 1}))["markdown"],
        input()["markdown"]
    );
    assert_eq!(
        management.tool("list_rules", json!({}))["reports"]["1"]["cleanup_state"],
        "purged"
    );
    assert_eq!(
        management.tool(
            "search_event_reports",
            json!({"search_strings": ["hidden timeout"]})
        )["reports"][0]["acceptance_id"],
        1
    );
    assert!(results[0].get("markdown").is_none());
    assert!(
        fs::read_to_string(fixture.home.join("docker-calls"))
            .unwrap()
            .contains("rm --force --volumes fixture-container")
    );
    assert!(
        fs::read_to_string(fixture.home.join("zellij-calls"))
            .unwrap()
            .contains("close-pane --pane-id terminal_100")
    );
    let unlinked = reader.tool("conclude", input());
    assert_eq!(
        unlinked,
        json!({"instance": "reader", "cleanup_state": "pending"})
    );
    reader.close(Some(libc::SIGKILL));
    wait_for(|| {
        !fixture.home.join("workspaces/reader").exists()
            && fixture.runtime_record("reader", "journal").is_none()
            && fixture.runtime_record("reader", "startup").is_none()
    });
    assert!(fixture.runtime_record("reader", "journal").is_none());
    assert!(fixture.runtime_record("reader", "startup").is_none());
    assert_eq!(
        management.tool(
            "search_event_reports",
            json!({"search_strings": ["incident"]})
        )["reports"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unlinked_conclusion_preserves_resources_on_close_failure_and_survives_retry_disconnection() {
    let fixture = fixture();
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    connection
        .execute("DELETE FROM rule_acceptances", [])
        .unwrap();
    fs::write(fixture.home.join("fail-close"), "").unwrap();
    fs::write(fixture.home.join("release-close"), "").unwrap();
    let mut agent = client(&fixture, "review");
    let mut reader = client(&fixture, "reader");
    assert_eq!(
        agent.tool("conclude", input()),
        json!({"instance": "review", "cleanup_state": "pending"})
    );
    wait_for(|| {
        String::from_utf8_lossy(
            &fixture
                .run(&["inspect-instance", "review", "--json"])
                .stdout,
        )
        .contains("fixture closure failed")
    });
    assert!(
        fixture
            .home
            .join("workspaces/review/app/file.txt")
            .is_file()
    );
    assert!(
        reader.tool("search_events", json!({"search_strings": ["incident"]}))["reports"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !fs::read_to_string(fixture.home.join("docker-calls"))
            .unwrap()
            .contains("rm --force")
    );
    fs::remove_file(fixture.home.join("fail-close")).unwrap();
    fs::remove_file(fixture.home.join("release-close")).unwrap();
    fs::remove_file(fixture.home.join("closing")).unwrap();
    agent.tool("conclude", input());
    wait_for(|| fixture.home.join("closing").exists());
    agent.close(Some(libc::SIGKILL));
    fs::write(fixture.home.join("release-close"), "").unwrap();
    wait_for(|| {
        !fixture.home.join("workspaces/review").exists()
            && fixture.runtime_record("review", "journal").is_none()
            && fixture.runtime_record("review", "startup").is_none()
    });
    assert!(fixture.home.join("workspaces/reader").is_dir());
    assert!(fixture.runtime_record("review", "journal").is_none());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM rule_acceptance_reports", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert!(
        fs::read_to_string(fixture.home.join("docker-calls"))
            .unwrap()
            .contains("rm --force --volumes fixture-container")
    );
    assert!(
        fs::read_to_string(fixture.home.join("zellij-calls"))
            .unwrap()
            .contains("close-pane --pane-id terminal_100")
    );
}

#[test]
fn instance_guidance_is_scoped_reread_and_preserved_across_restarts() {
    let fixture = fixture();
    let mut agent = client(&fixture, "review");
    assert!(
        agent.info["instructions"]
            .as_str()
            .unwrap()
            .contains("Call get_instructions before")
    );
    let guidance = agent.tool("get_instructions", json!({}));
    assert_eq!(guidance["instance"], "review");
    assert_eq!(guidance["template"], "website");
    assert_eq!(guidance["namespace"], "cli-test");
    assert_eq!(
        guidance["workspace"],
        fixture.home.join("workspaces/review").display().to_string()
    );
    assert_eq!(
        guidance["core_guidance"],
        include_str!("../../../instance-core-guidance.md")
    );
    assert_eq!(
        guidance["markdown"],
        include_str!("../../../instance-agent-instructions.md")
    );
    let file = fixture.home.join("instance-instructions.md");
    assert_eq!(guidance["file"], file.display().to_string());
    fs::write(
        &file,
        "# Local policy\nPreserve evidence before concluding.\n",
    )
    .unwrap();
    assert_eq!(
        agent.tool("get_instructions", json!({}))["markdown"],
        fs::read_to_string(&file).unwrap()
    );
    agent.close(None);
    let mut restarted = client(&fixture, "reader");
    let reread = restarted.tool("get_instructions", json!({}));
    assert_eq!(reread["instance"], "reader");
    assert_eq!(reread["markdown"], fs::read_to_string(&file).unwrap());
    assert_eq!(reread["core_guidance"], guidance["core_guidance"]);
    fs::remove_file(&file).unwrap();
    let error = restarted.request(
        "tools/call",
        json!({"name": "get_instructions", "arguments": {}}),
    );
    assert_eq!(error["isError"], true);
}

#[test]
fn failed_client_closure_preserves_the_report_and_resources_and_identical_retry_completes() {
    let fixture = fixture();
    fs::write(fixture.home.join("fail-close"), "").unwrap();
    fs::write(fixture.home.join("release-close"), "").unwrap();
    let mut agent = client(&fixture, "review");
    let mut reader = client(&fixture, "reader");
    agent.tool("conclude", input());
    wait_for(|| {
        reader.tool("get_event_report", json!({"acceptance_id": 1}))["cleanup_state"] == "failed"
    });
    let saved = reader.tool("get_event_report", json!({"acceptance_id": 1}));
    assert!(
        saved["cleanup_error"]
            .as_str()
            .unwrap()
            .contains("fixture closure failed")
    );
    assert_eq!(saved["markdown"], input()["markdown"]);
    assert!(
        fixture
            .home
            .join("workspaces/review/app/file.txt")
            .is_file()
    );
    assert!(
        !fs::read_to_string(fixture.home.join("docker-calls"))
            .unwrap()
            .contains("rm --force")
    );
    let mut replacement = input();
    replacement["title"] = json!("Replacement");
    let error = agent.request(
        "tools/call",
        json!({"name": "conclude", "arguments": replacement}),
    );
    assert_eq!(error["isError"], true);
    fs::remove_file(fixture.home.join("fail-close")).unwrap();
    agent.tool("conclude", input());
    wait_for(|| {
        reader.tool("get_event_report", json!({"acceptance_id": 1}))["cleanup_state"] == "purged"
    });
    let completed = reader.tool("get_event_report", json!({"acceptance_id": 1}));
    assert_eq!(completed["reported_at"], saved["reported_at"]);
    assert_eq!(completed["markdown"], saved["markdown"]);
    assert!(!fixture.home.join("workspaces/review").exists());
}

const ZELLIJ: &str = r#"#!/bin/sh
case "$*" in
  *close-pane*)
    printf '%s\n' "$*" >> "$TANDEM_HOME/zellij-calls"
    test -d "$TANDEM_HOME/workspaces/review" || exit 1
    touch "$TANDEM_HOME/closing"
    count=0
    while [ ! -f "$TANDEM_HOME/release-close" ]; do
      count=$((count + 1))
      if [ "$count" -gt 200 ]; then exit 2; fi
      sleep 0.05
    done
    if [ -f "$TANDEM_HOME/fail-close" ]; then printf 'fixture closure failed\n' >&2; exit 3; fi
    touch "$TANDEM_HOME/closed"
    ;;
  *list-panes*)
    if [ -f "$TANDEM_HOME/closed" ]; then printf '[]'; else cat "$TANDEM_HOME/panes.json"; fi
    ;;
  *) exec "$(dirname "$0")/zellij-base" "$@";;
esac
"#;
