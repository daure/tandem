use super::*;

#[test]
fn stream_lifecycle_waits_for_collector_acknowledgment_and_preserves_siblings() {
    let fixture = provider_fixture();
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    let manifest = json!({
        "schema_version": 1, "name": "dev-message", "profile": "message",
        "description": "Fixture", "protocol": "tandem-events-v1",
        "streams": ["samples", "sibling"], "stream_control": true,
    });
    fs::write(
        fixture
            .home
            .join("templates/providers/message/provider.json"),
        manifest.to_string(),
    )
    .unwrap();
    connection
        .execute(
            "UPDATE provider_launches SET manifest = ?1 WHERE name = 'message'",
            [manifest.to_string()],
        )
        .unwrap();
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client
        .get(format!("http://{address}/v1/identity"))
        .send()
        .is_ok_and(|response| response.status().is_success())
    {
        assert!(Instant::now() < deadline, "sidecar failed to start");
        thread::sleep(Duration::from_millis(20));
    }
    let token = "a".repeat(64);
    connection.execute("INSERT INTO event_providers(namespace, name, token) VALUES ('cli-test', 'dev-message', ?1)", [&token]).unwrap();
    let output = fixture.run(&["provider", "start", "message"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let controls_url = format!("http://{address}/v1/streams");
    let read_controls = || -> Vec<serde_json::Value> {
        let text = client
            .get(&controls_url)
            .bearer_auth(&token)
            .send()
            .unwrap()
            .text()
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["streams"]
            .as_array()
            .unwrap()
            .clone()
    };
    let acknowledge = |control: &serde_json::Value| {
        assert_eq!(
            client
                .post(format!("{controls_url}/ack"))
                .bearer_auth(&token)
                .header("Content-Type", "application/json")
                .body(control.to_string())
                .send()
                .unwrap()
                .status(),
            204
        );
    };
    for control in read_controls() {
        acknowledge(&control);
    }
    let run_stream = |stream: &str, action: &str, enabled: bool| {
        let previous = read_controls()
            .into_iter()
            .find(|row| row["stream"] == stream)
            .map(|row| row["revision"].as_i64().unwrap())
            .unwrap_or(0);
        let mut child = fixture
            .command(&["provider", action, "message", "--stream", stream])
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let control = loop {
            if let Some(control) = read_controls().into_iter().find(|control| {
                control["stream"] == stream
                    && control["enabled"] == enabled
                    && control["revision"].as_i64().unwrap() > previous
            }) {
                break control;
            }
            assert!(Instant::now() < deadline, "stream control did not publish");
            thread::sleep(Duration::from_millis(20));
        };
        assert!(
            child.try_wait().unwrap().is_none(),
            "stream lifecycle completed before acknowledgment"
        );
        acknowledge(&control);
        let output = wait_output(child);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("live-only"));
    };
    let inspect = || -> serde_json::Value {
        let output = fixture.run(&["list-providers"]);
        let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        snapshot["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|provider| provider["name"] == "message")
            .unwrap()
            .clone()
    };
    for (action, enabled) in [("stop", false), ("start", true)] {
        run_stream("samples", action, enabled);
        let provider = inspect();
        assert_eq!(provider["status"], "running");
        let streams = provider["streams"].as_array().unwrap();
        assert_eq!(
            streams[0]["status"],
            if enabled { "running" } else { "stopped" }
        );
        assert_eq!(streams[1]["status"], "running");
        assert_eq!(streams[1]["enabled"], true);
    }
    let commands = fs::read_to_string(fixture.home.join("provider-commands")).unwrap();
    assert!(!commands.lines().any(|line| line.starts_with("stop ")));
    run_stream("samples", "stop", false);
    run_stream("sibling", "stop", false);
    let provider = inspect();
    assert_eq!(provider["status"], "stopped");
    assert!(
        provider["streams"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["enabled"] == false)
    );
    run_stream("samples", "start", true);
    let provider = inspect();
    assert_eq!(provider["status"], "running");
    assert_eq!(provider["streams"][0]["status"], "running");
    assert_eq!(provider["streams"][1]["enabled"], false);
    fs::write(fixture.home.join("provider-paused"), "").unwrap();
    run_stream("samples", "start", true);
    assert_eq!(inspect()["status"], "running");
    assert!(!fixture.home.join("provider-paused").exists());
    assert_eq!(inspect()["streams"][1]["enabled"], false);
    let output = fixture.run(&["provider", "start", "message"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let provider = inspect();
    assert!(
        provider["streams"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "starting")
    );
    for control in read_controls() {
        acknowledge(&control);
    }
    assert!(
        inspect()["streams"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "running")
    );
    // An uninstalled collector starts with only the requested stream enabled.
    fs::write(fixture.home.join("provider-absent"), "").unwrap();
    connection
        .execute("DELETE FROM provider_launches WHERE name = 'message'", [])
        .unwrap();
    run_stream("samples", "start", true);
    let provider = inspect();
    assert_eq!(provider["status"], "running");
    assert_eq!(provider["streams"][0]["enabled"], true);
    assert_eq!(provider["streams"][1]["enabled"], false);
}
