use super::*;

struct Sidecar(Child);

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn acceptances(fixture: &Fixture) -> Vec<serde_json::Value> {
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    connection.busy_timeout(Duration::from_secs(5)).unwrap();
    connection
        .prepare("SELECT payload FROM rule_acceptances ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect()
}

#[test]
fn dispatch_waits_for_readiness_and_retains_independent_literal_prompt_outcomes() {
    for start_instance in [true, false] {
        dispatch(start_instance);
    }
}

fn dispatch(start_instance: bool) {
    let fixture = Fixture::new();
    if start_instance {
        fs::remove_file(fixture.home.join("templates/website/compose.yaml")).unwrap();
    }
    assert!(fixture.run(&["list-templates", "--json"]).status.success());
    let token = "a".repeat(64);
    let authorization = format!("Bearer {token}");
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO event_providers(namespace, name, token) VALUES ('cli-test', 'sample', ?1)",
            [&token],
        )
        .unwrap();
    for (name, prompt) in [
        ("inspect", "Inspect {{event.data.text}}"),
        ("missing", "{{event.metadata.missing}}"),
    ] {
        let mut definition = json!({
            "name": name, "description": "Event fixture", "script": "fn matches(event) { true }",
            "template": "website", "model": "openai/test", "variant": "fast", "initial_prompt": prompt,
            "enabled": true, "start_instance": start_instance
        });
        if !start_instance {
            definition["focus_pane"] = json!(false);
        }
        let mut recipe = definition.clone();
        recipe.as_object_mut().unwrap().remove("enabled");
        let directory = fixture.home.join("templates/rules").join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("rule.json"), recipe.to_string()).unwrap();
        connection.execute("INSERT INTO event_rules(namespace, name, revision, definition, zellij_session, enabled) VALUES ('cli-test', ?1, 1, ?2, 'main', 1)", rusqlite::params![name, definition.to_string()]).unwrap();
    }
    drop(connection);
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let _sidecar = Sidecar(
        fixture
            .command(&["provider-sidecar-worker", "--bind", &address.to_string()])
            .spawn()
            .unwrap(),
    );
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let origin = format!("http://{address}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if client
            .get(format!("{origin}/v1/identity"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            break;
        }
        assert!(Instant::now() < deadline, "sidecar failed to start");
        thread::sleep(Duration::from_millis(20));
    }
    let untrusted = "'; $(touch injected)\n{{event.secret}}";
    let batch = json!({"events": [{
        "schema_version": 1, "event_id": "one", "stream": "fixture", "profile": "message",
        "type": "fixture.message", "summary": "Inspect fixture",
        "data": {"author": "Test", "channel": "Fixture", "text": untrusted}
    }]})
    .to_string();
    let ingest = || {
        let response = client
            .post(format!("{origin}/v1/events"))
            .header("Authorization", &authorization)
            .header("Content-Type", "application/json")
            .body(batch.clone())
            .send()
            .unwrap();
        let status = response.status();
        let body = response.text().unwrap();
        assert!(status.is_success(), "{status}: {body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    };
    assert_eq!(ingest()["receipts"][0]["duplicate"], false);
    let deadline = Instant::now() + Duration::from_secs(20);
    let success = loop {
        let rows = acceptances(&fixture);
        if rows.len() == 2 && fixture.home.join("opened").exists() {
            let success = rows
                .iter()
                .find(|row| row["rule_name"] == "inspect")
                .unwrap()
                .clone();
            let workspace = fixture
                .home
                .join("workspaces")
                .join(success["instance"].as_str().unwrap());
            assert!(workspace.join("app/file.txt").is_file());
            assert!(workspace.join("AGENTS.md").is_file());
            assert_eq!(
                fixture
                    .runtime_record(success["instance"].as_str().unwrap(), "startup")
                    .unwrap()["operation"]["state"],
                "succeeded"
            );
            let failed = rows
                .iter()
                .find(|row| row["rule_name"] == "missing")
                .unwrap();
            assert_eq!(failed["status"], "failed");
            assert!(
                failed["error"]
                    .as_str()
                    .unwrap()
                    .contains("missing prompt field")
            );
            break success;
        }
        assert!(Instant::now() < deadline, "dispatch failed: {rows:#?}");
        thread::sleep(Duration::from_millis(20));
    };
    let args = fs::read_to_string(fixture.home.join("opencode-args")).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.home.join("branch"))
            .unwrap()
            .trim(),
        success["instance"].as_str().unwrap()
    );
    assert_eq!(
        args.split_terminator('\0').collect::<Vec<_>>(),
        ["--model", "openai/test", "--variant", "fast"]
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened-parameters")).unwrap(),
        format!("Inspect {untrusted}")
    );
    assert_eq!(
        success["rule"]["definition"]["start_instance"],
        start_instance
    );
    assert_eq!(success["rule"]["definition"]["variant"], "fast");
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened-model")).unwrap(),
        "openai/test#fast"
    );
    let instructions = fs::read_to_string(fixture.home.join("opened-instructions")).unwrap();
    assert!(
        instructions.contains(if start_instance {
            "workspace-only"
        } else {
            "Services won't start automatically"
        }),
        "{instructions}"
    );
    assert!(!fixture.home.join("started").exists());
    let presence = fixture.home.join("state/tandem/opencode");
    fs::create_dir_all(&presence).unwrap();
    fs::write(presence.join("rule.json"), json!({
        "pid": std::process::id(), "observed_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
        "id": "ses_rule_fixture", "title": "Fixture", "activity": "idle", "server": "",
        "directory": fixture.home.join("workspaces").join(success["instance"].as_str().unwrap()),
        "zellij_session": "main", "pane_id": 100
    }).to_string()).unwrap();
    loop {
        let rows = acceptances(&fixture);
        if rows
            .iter()
            .any(|row| row["status"] == "launched" && row["session_id"] == "ses_rule_fixture")
        {
            break;
        }
        assert!(Instant::now() < deadline, "session link failed: {rows:#?}");
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(ingest()["receipts"][0]["duplicate"], true);
    assert_eq!(success["rule"]["definition"]["focus_pane"], start_instance);
    let calls = fs::read_to_string(fixture.home.join("zellij-calls")).unwrap();
    assert_eq!(calls.contains("--no-focus"), !start_instance, "{calls}");
    assert_eq!(
        calls.contains("focus-pane-id terminal_100"),
        start_instance,
        "{calls}"
    );
    assert_eq!(
        calls.contains("go-to-tab-by-id 9"),
        start_instance,
        "{calls}"
    );
    assert_eq!(acceptances(&fixture).len(), 2);
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let response = client
        .get(format!("{origin}/v1/notifications"))
        .header("Authorization", &authorization)
        .send()
        .unwrap();
    let notifications: serde_json::Value = serde_json::from_str(&response.text().unwrap()).unwrap();
    let assigned: Vec<_> = notifications["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == "assigned")
        .collect();
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0]["dispatch_id"], success["id"]);
    assert_eq!(assigned[0]["instance"], success["instance"]);
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    let retained = || {
        let text: String = connection
            .query_row(
                "SELECT payload FROM rule_acceptance_workspaces WHERE acceptance_id = ?1",
                [success["id"].as_i64().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&text).unwrap()
    };
    assert_eq!(retained()["sessions"][0]["id"], "ses_rule_fixture");
    assert_eq!(retained()["sessions"][0]["panes"], json!([]));
    fs::remove_file(presence.join("rule.json")).unwrap();
    let mut command = fixture.command(&["mcp-instance"]);
    command.current_dir(
        fixture
            .home
            .join("workspaces")
            .join(success["instance"].as_str().unwrap()),
    );
    let mut agent = super::startup::Client::with_command(command);
    let report = json!({"title":"Fixture inspected", "summary":"Literal prompt verified", "markdown":"# Evidence\nThe fixture completed."});
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let result = agent.request(
            "tools/call",
            json!({"name":"conclude", "arguments": report}),
        );
        if result.to_string().contains("rule dispatch is busy") {
            assert!(Instant::now() < deadline, "{result}");
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        assert_ne!(result["isError"], true, "{result}");
        break;
    }
    let mut management = super::startup::Client::with_command(fixture.command(&["mcp"]));
    loop {
        let saved = management.tool("get_event_report", json!({"acceptance_id": success["id"]}));
        if saved["cleanup_state"] == "purged" {
            break;
        }
        assert_ne!(saved["cleanup_state"], "failed", "{saved}");
        assert!(Instant::now() < deadline, "{saved}");
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(retained()["sessions"][0]["id"], "ses_rule_fixture");
    assert!(
        !fixture
            .home
            .join("workspaces")
            .join(success["instance"].as_str().unwrap())
            .exists()
    );
    assert_eq!(
        management.tool(
            "search_event_reports",
            json!({"search_strings":["fixture completed"]})
        )["reports"][0]["acceptance_id"],
        success["id"]
    );
}
