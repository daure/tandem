use std::os::unix::fs::MetadataExt;

use super::*;

struct ReceiverCleanup(PathBuf);

impl Drop for ReceiverCleanup {
    fn drop(&mut self) {
        let Ok(source) = fs::read_to_string(&self.0) else {
            return;
        };
        let Ok(receipt) = serde_json::from_str::<serde_json::Value>(&source) else {
            return;
        };
        let Some(pid) = receipt["pid"].as_u64() else {
            return;
        };
        if identity(receipt["origin"].as_str().unwrap()).as_ref() == Some(&receipt) {
            unsafe { libc::kill(pid as i32, libc::SIGINT) };
        }
    }
}

fn identity(origin: &str) -> Option<serde_json::Value> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let source = client
        .get(format!("{origin}/v1/identity"))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .ok()?;
    serde_json::from_str(&source).ok()
}

fn start_receiver(
    fixture: &Fixture,
    executable: &Path,
) -> (Sidecar, ReceiverCleanup, String, serde_json::Value) {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", reservation.local_addr().unwrap());
    let address = reservation.local_addr().unwrap().to_string();
    drop(reservation);
    let worker = Sidecar(
        fixture
            .command_at(executable, &["provider-sidecar-worker", "--bind", &address])
            .spawn()
            .unwrap(),
    );
    let cleanup = ReceiverCleanup(fixture.home.join("locks/provider-sidecar-cli-test.json"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let receipt = loop {
        if let Some(receipt) = identity(&origin) {
            break receipt;
        }
        assert!(Instant::now() < deadline, "receiver failed to start");
        thread::sleep(Duration::from_millis(20));
    };
    (worker, cleanup, origin, receipt)
}

#[test]
fn provider_start_replaces_an_outdated_receiver_on_its_retained_address() {
    let fixture = provider_fixture();
    let executable = fixture.bin.join("tandem");
    fs::hard_link(env!("CARGO_BIN_EXE_tandem"), &executable).unwrap();
    let (mut old, _cleanup, origin, original) = start_receiver(&fixture, &executable);
    let token = "a".repeat(64);
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO event_providers(namespace, name, token) VALUES ('cli-test', 'sample', ?1)",
            [&token],
        )
        .unwrap();
    let definition = json!({
        "name": "observe", "description": "Receiver fixture",
        "script": "fn matches(event) { false }", "template": "website",
        "model": "openai/test", "initial_prompt": "Inspect {{event}}", "enabled": true
    });
    let mut recipe = definition.clone();
    recipe.as_object_mut().unwrap().remove("enabled");
    let directory = fixture.home.join("templates/rules/observe");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("rule.json"), recipe.to_string()).unwrap();
    connection.execute("INSERT INTO event_rules(namespace, name, revision, definition, zellij_session, enabled) VALUES ('cli-test', 'observe', 1, ?1, 'main', 1)", [definition.to_string()]).unwrap();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let ingest = |id: &str| {
        let batch = json!({"events": [{
            "schema_version": 1, "event_id": id, "stream": "fixture", "profile": "message",
            "type": "fixture.message", "summary": "Inspect fixture",
            "data": {"author": "Test", "channel": "Fixture", "text": "Retained work"}
        }]});
        let response = client
            .post(format!("{origin}/v1/events"))
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .body(batch.to_string())
            .send()
            .unwrap();
        let status = response.status();
        let body = response.text().unwrap();
        assert!(status.is_success(), "{status}: {body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    };
    assert_eq!(ingest("before")["receipts"][0]["duplicate"], false);
    let mut old_client =
        super::super::startup::Client::with_command(fixture.command_at(&executable, &["mcp"]));
    let replacement = fixture.bin.join("replacement");
    fs::copy(env!("CARGO_BIN_EXE_tandem"), &replacement).unwrap();
    fs::rename(replacement, &executable).unwrap();

    let output = wait_output(
        fixture
            .command_at(&executable, &["provider", "start", "message"])
            .spawn()
            .unwrap(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let current = identity(&origin).unwrap();
    assert_ne!(current["pid"], original["pid"]);
    assert_ne!(current["identity"], original["identity"]);
    assert_eq!(current["origin"], original["origin"]);
    assert_eq!(
        fs::metadata(format!("/proc/{}/exe", current["pid"].as_u64().unwrap()))
            .unwrap()
            .ino(),
        fs::metadata(&executable).unwrap().ino()
    );
    assert!(old.0.try_wait().unwrap().unwrap().success());
    assert_eq!(ingest("before")["receipts"][0]["duplicate"], true);
    assert_eq!(ingest("after")["receipts"][0]["duplicate"], false);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM rule_evaluations WHERE rule_name = 'observe'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    let output = fixture
        .command_at(&executable, &["provider", "start", "message"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(identity(&origin).unwrap(), current);
    old_client.tool(
        "provider_action",
        json!({"name": "message", "action": "start", "confirmed": true}),
    );
    assert_eq!(identity(&origin).unwrap(), current);
}

#[test]
fn provider_start_preserves_a_receiver_when_ownership_cannot_be_verified() {
    let fixture = provider_fixture();
    let (mut worker, cleanup, origin, original) =
        start_receiver(&fixture, Path::new(env!("CARGO_BIN_EXE_tandem")));
    let mut altered = original.clone();
    altered["identity"] = json!("unverified");
    fs::write(&cleanup.0, altered.to_string()).unwrap();
    let output = fixture.run(&["provider", "start", "message"]);
    fs::write(&cleanup.0, original.to_string()).unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("lease is held but readiness cannot be verified")
    );
    assert!(worker.0.try_wait().unwrap().is_none());
    assert_eq!(identity(&origin).unwrap(), original);
}
