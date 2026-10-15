use super::startup::Client;
use super::*;

#[path = "instance_close.rs"]
mod closing;

#[path = "memories.rs"]
mod memories;

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
    // Purge closes associated clients even with observation disabled.
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
        "id": "ses_incident", "title": "Incident", "directory": fixture.home.join("workspaces/website/review"),
        "server": "", "activity": "idle", "zellij_session": "main", "pane_id": 100
    }).to_string()).unwrap();
    fixture
}

fn client(fixture: &Fixture, name: &str) -> Client {
    let mut command = fixture.command(&["mcp-instance"]);
    command
        .current_dir(
            fixture
                .home
                .join("workspaces")
                .join(if name == "reader" { "blank" } else { "website" })
                .join(name),
        )
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
fn scoped_purge_survives_mcp_kill_and_preserves_memory_after_resource_deletion() {
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
            "close",
            "get_conclusion",
            "get_event_report",
            "get_instructions",
            "get_memory",
            "ping",
            "purge",
            "remember",
            "search_conclusions",
            "search_events",
            "search_memories",
            "start_self",
            "stop_self",
            "update_repositories"
        ]
    );
    let injection = agent.request_raw(
        "tools/call",
        json!({"name": "purge", "arguments": {"name": "reader"}}),
    );
    assert_eq!(injection["error"]["code"], -32602);
    let memory = agent.tool("remember", input());
    let receipt = agent.tool("purge", json!({}));
    assert_eq!(receipt["acceptance_id"], 1);
    assert_eq!(receipt["cleanup_state"], "pending");
    wait_for(|| fixture.home.join("closing").exists());
    let mut reader = client(&fixture, "reader");
    assert_eq!(
        reader.tool("get_memory", json!({"memory_id": memory["memory_id"]}))["markdown"],
        input()["markdown"]
    );
    assert!(fixture.home.join("workspaces/website/review").is_dir());
    agent.close(Some(libc::SIGKILL));
    fs::write(fixture.home.join("release-close"), "").unwrap();
    wait_for(|| {
        reader.tool(
            "get_conclusion",
            json!({"conclusion_id": receipt["conclusion_id"]}),
        )["cleanup_state"]
            == "purged"
    });
    assert!(!fixture.home.join("workspaces/website/review").exists());
    assert!(fixture.home.join("workspaces/blank/reader").is_dir());
    assert!(fixture.runtime_record("review", "journal").is_none());
    assert!(fixture.runtime_record("review", "startup").is_none());
    let results =
        reader.tool("search_memories", json!({"query": "HIDDEN TIMEOUT"}))["memories"].clone();
    assert_eq!(results.as_array().unwrap().len(), 1);
    assert_eq!(results[0]["title"], "Incident resolved");
    assert_eq!(results[0]["summary"], "Verified service recovery");
    let mut management = Client::with_command(fixture.command(&["mcp"]));
    assert_eq!(
        management.tool("get_memory", json!({"memory_id": memory["memory_id"]}))["markdown"],
        input()["markdown"]
    );
    assert_eq!(
        management.tool(
            "get_conclusion",
            json!({"conclusion_id": receipt["conclusion_id"]})
        )["cleanup_state"],
        "purged"
    );
    assert_eq!(
        management.tool("search_memories", json!({"query": "hidden timeout"}))["memories"][0]["acceptance_id"],
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
    let unlinked = reader.tool("purge", json!({}));
    assert_eq!(unlinked["instance"], "reader");
    assert_eq!(unlinked["cleanup_state"], "pending");
    assert!(unlinked["conclusion_id"].as_i64().unwrap() > 0);
    reader.close(Some(libc::SIGKILL));
    wait_for(|| {
        !fixture.home.join("workspaces/blank/reader").exists()
            && fixture.runtime_record("reader", "journal").is_none()
            && fixture.runtime_record("reader", "startup").is_none()
    });
    assert!(fixture.runtime_record("reader", "journal").is_none());
    assert!(fixture.runtime_record("reader", "startup").is_none());
    assert_eq!(
        management.tool("search_memories", json!({"query": "incident"}))["memories"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        management.tool(
            "get_conclusion",
            json!({"conclusion_id": unlinked["conclusion_id"]})
        )["report"],
        serde_json::Value::Null
    );
}

#[test]
fn purge_supports_acceptances_and_manual_instances_and_reuses_cleanup_on_retry() {
    let fixture = fixture();
    fs::write(fixture.home.join("release-close"), "").unwrap();
    fs::write(fixture.home.join("fail-close"), "").unwrap();
    let mut agent = client(&fixture, "review");
    let mut management = Client::with_command(fixture.command(&["mcp"]));
    let receipt = agent.tool("purge", json!({}));
    assert_eq!(receipt["acceptance_id"], 1);
    wait_for(|| {
        management.tool(
            "get_conclusion",
            json!({"conclusion_id": receipt["conclusion_id"]}),
        )["cleanup_state"]
            == "failed"
    });
    let no_report = management.request(
        "tools/call",
        json!({"name":"get_event_report", "arguments":{"acceptance_id":1}}),
    );
    assert_eq!(no_report["isError"], true);
    fs::remove_file(fixture.home.join("fail-close")).unwrap();
    let retry = agent.tool("purge", json!({}));
    assert_eq!(retry["conclusion_id"], receipt["conclusion_id"]);
    wait_for(|| {
        management.tool(
            "get_conclusion",
            json!({"conclusion_id": receipt["conclusion_id"]}),
        )["cleanup_state"]
            == "purged"
    });
    assert_eq!(
        management.tool(
            "get_conclusion",
            json!({"conclusion_id": receipt["conclusion_id"]})
        )["report"],
        serde_json::Value::Null
    );
    let mut reader = client(&fixture, "reader");
    let manual = reader.tool("purge", json!({}));
    assert_eq!(manual["acceptance_id"], serde_json::Value::Null);
    wait_for(|| {
        management.tool(
            "get_conclusion",
            json!({"conclusion_id": manual["conclusion_id"]}),
        )["cleanup_state"]
            == "purged"
    });
    let mut command = fixture.command(&["mcp"]);
    command.env("TANDEM_NAMESPACE", "other");
    let mut other = Client::with_command(command);
    assert_eq!(
        other.request(
            "tools/call",
            json!({"name":"get_conclusion", "arguments":{"conclusion_id": manual["conclusion_id"]}})
        )["isError"],
        true
    );
}

#[test]
fn unlinked_purge_preserves_resources_on_close_failure_and_survives_retry_disconnection() {
    let fixture = fixture();
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    connection
        .execute("DELETE FROM rule_acceptances", [])
        .unwrap();
    fs::write(fixture.home.join("fail-close"), "").unwrap();
    fs::write(fixture.home.join("release-close"), "").unwrap();
    let mut agent = client(&fixture, "review");
    let mut reader = client(&fixture, "reader");
    let memory = agent.tool("remember", input());
    let receipt = agent.tool("purge", json!({}));
    assert_eq!(receipt["instance"], "review");
    assert_eq!(receipt["cleanup_state"], "pending");
    wait_for(|| {
        String::from_utf8_lossy(
            &fixture
                .run(&["inspect-instance", "review", "--json"])
                .stdout,
        )
        .contains("fixture closure failed")
    });
    let retained = reader.tool(
        "get_conclusion",
        json!({"conclusion_id": receipt["conclusion_id"]}),
    );
    assert_eq!(retained["report"], serde_json::Value::Null);
    assert_eq!(retained["cleanup_state"], "failed");
    assert_eq!(
        reader.tool("search_memories", json!({"query": "hidden timeout"}))["memories"][0]["memory_id"],
        memory["memory_id"]
    );
    assert!(
        fixture
            .home
            .join("workspaces/website/review/app/file.txt")
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
    agent.tool("purge", json!({}));
    wait_for(|| fixture.home.join("closing").exists());
    agent.close(Some(libc::SIGKILL));
    fs::write(fixture.home.join("release-close"), "").unwrap();
    wait_for(|| {
        !fixture.home.join("workspaces/website/review").exists()
            && fixture.runtime_record("review", "journal").is_none()
            && fixture.runtime_record("review", "startup").is_none()
    });
    assert!(fixture.home.join("workspaces/blank/reader").is_dir());
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
        fixture
            .home
            .join("workspaces/website/review")
            .display()
            .to_string()
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
    fs::write(&file, "# Local policy\nPreserve evidence before purge.\n").unwrap();
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

const ZELLIJ: &str = r#"#!/bin/sh
case "$*" in
  *close-pane*|*close-tab*)
    printf '%s\n' "$*" >> "$TANDEM_HOME/zellij-calls"
    test -d "$TANDEM_HOME/workspaces/website/review" || exit 1
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
