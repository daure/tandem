use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

struct Companion {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

const PREFLIGHT_REFUSAL: &str = "OpenCode tab action is already in progress";

impl Companion {
    fn start(fixture: &Fixture) -> Self {
        Self::with_receipt_error(fixture, None)
    }

    fn with_receipt_error(fixture: &Fixture, error: Option<&'static str>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = format!("http://{}", listener.local_addr().unwrap());
        let directory = fixture.home.join("workspaces/website/review");
        let root = fixture.home.join("state/tandem/opencode");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("client.json"), json!({
            "pid":std::process::id(), "observed_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            "id":"", "title":"OpenCode", "directory":directory, "server":server,
            "activity":"idle", "zellij_session":"main", "pane_id":100,
            "tab_control":{"server":server,"token":"client-secret","prompted_sessions":true,"session_prompts":true,"session_tabs":true}
        }).to_string()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            let mut sessions = Vec::<serde_json::Value>::new();
            while !stopped.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0usize;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("content-length: ") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let body: serde_json::Value = if bytes.is_empty() {
                    json!({})
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                let response = match path.as_str() {
                    "/sessions" if error == Some(PREFLIGHT_REFUSAL) => json!({"session_id":null,"server":server,"prompt_outcome":"not_submitted","creation_outcome":"not_created","error":PREFLIGHT_REFUSAL}),
                    "/sessions" => {
                        let id = format!("ses_created_{}", sessions.len() + 1);
                        sessions.push(json!({"id":id,"title":format!("Conversation {id}"),"location":{"directory":directory},"time":{"created":1,"updated":2}}));
                        if let Some(path) = body["identity_receipt"].as_str() {
                            let mut identity: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
                            identity["session_id"] = json!(id);
                            identity["server"] = json!(server);
                            fs::write(path, identity.to_string()).unwrap();
                        }
                        json!({"session_id":id,"server":server,"prompt_outcome":if body["initial_prompt"].is_null() { "not_submitted" } else { "submitted" },"error":error})
                    }
                    "/sessions/prompt" => json!({"session_id":body["sessionID"],"server":server,"prompt_outcome":"queued","error":null}),
                    "/api/info" => json!({"version":"2.0.22"}),
                    "/api/session/active" => json!({"data":{}}),
                    path if path.starts_with("/api/session?") => json!({"data":sessions}),
                    path if path.starts_with("/api/form?") || path.starts_with("/api/permission/request?") => json!({"data":[]}),
                    path if path.ends_with("/export") => json!({"data":{"messages":[]}}),
                    path if path.starts_with("/api/session/") => json!({"data":sessions.iter().find(|session| path.ends_with(session["id"].as_str().unwrap())).unwrap()}),
                    _ => panic!("unexpected test endpoint: {path}"),
                }.to_string();
                let status = if path == "/sessions" && error == Some(PREFLIGHT_REFUSAL) {
                    "409 Conflict"
                } else {
                    "200 OK"
                };
                let _ = stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes());
            }
        });
        Self {
            stop,
            thread: Some(worker),
        }
    }
}

#[test]
fn accepted_input_with_receipt_errors_exits_nonzero_and_preserves_the_receipt() {
    let fixture = Fixture::new();
    let _companion = Companion::with_receipt_error(&fixture, Some("Zellij focus failed"));
    for arguments in [
        vec![
            "new-instance",
            "review",
            "-t",
            "website",
            "--opencode",
            "Initial task",
            "--json",
        ],
        vec!["new-instance-session", "review", "Another task", "--json"],
    ] {
        let output = fixture.run(&arguments);
        assert!(!output.status.success());
        let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            receipt["session_id"]
                .as_str()
                .unwrap()
                .starts_with("ses_created_")
        );
        assert_eq!(receipt["prompt_outcome"], "submitted");
        assert_eq!(receipt["error"], "Zellij focus failed");
    }
}

#[test]
fn confirmed_preflight_refusal_releases_the_publication_gate() {
    let fixture = Fixture::new();
    let _companion = Companion::with_receipt_error(&fixture, Some(PREFLIGHT_REFUSAL));
    let output = fixture.run(&[
        "new-instance",
        "review",
        "-t",
        "website",
        "--opencode",
        "Initial task",
        "--origin",
        "initial",
        "--json",
    ]);
    assert!(!output.status.success());
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["session_id"], serde_json::Value::Null);
    assert_eq!(receipt["prompt_outcome"], "not_submitted");
    let connection = rusqlite::Connection::open(fixture.home.join("state-stream.sqlite3")).unwrap();
    let pending: u64 = connection.query_row("SELECT count(*) FROM session_creation_requests WHERE namespace = 'cli-test' AND session_id IS NULL", [], |row| row.get(0)).unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn listings_recover_creation_origins_from_private_identity_handoffs() {
    let fixture = Fixture::new();
    let _companion = Companion::start(&fixture);
    let created = fixture.run(&[
        "new-instance",
        "review",
        "-t",
        "website",
        "--opencode",
        "--json",
    ]);
    assert!(created.status.success());
    let receipt: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    let server = receipt["server"].as_str().unwrap();
    let directory = fixture.home.join("workspaces/website/review");
    let connection = rusqlite::Connection::open(fixture.home.join("state-stream.sqlite3")).unwrap();
    connection
        .execute("DELETE FROM session_creation_requests", [])
        .unwrap();
    connection.execute("INSERT INTO session_creation_requests(namespace, id, directory, instance, origin, created_at) VALUES ('cli-test', 'pending-handoff', ?1, 'review', 'initial-origin', 1)", [directory.to_str().unwrap()]).unwrap();
    let identities = fixture.home.join("runtime/cli-test/session-identities");
    fs::create_dir_all(&identities).unwrap();
    let identity = identities.join("pending-handoff.json");
    fs::write(&identity, json!({"request_id":"pending-handoff", "directory": directory, "session_id":"ses_created_1", "server":server}).to_string()).unwrap();
    fs::set_permissions(&identity, fs::Permissions::from_mode(0o600)).unwrap();
    let listing = fixture.run(&["list-sessions", "--json", "--include-closed"]);
    assert!(
        listing.status.success(),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    let listing: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert_eq!(listing["sessions"][0]["origins"], json!(["initial-origin"]));
    assert!(!identity.exists());
}

impl Drop for Companion {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn origins_round_trip_through_instance_creation_fresh_sessions_prompts_and_listings() {
    let fixture = Fixture::new();
    let _companion = Companion::start(&fixture);
    let created = fixture.run(&[
        "new-instance",
        "review",
        "-t",
        "website",
        "--opencode",
        "Initial task",
        "--origin",
        "slack://message/1",
        "--json",
    ]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let first: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    assert_eq!(first["session_id"], "ses_created_1");
    assert_eq!(first["prompt_outcome"], "submitted");
    let opened = fixture.run(&[
        "new-instance-session",
        "review",
        "Second task",
        "--origin",
        "slack://message/2",
        "--json",
    ]);
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    let second: serde_json::Value = serde_json::from_slice(&opened.stdout).unwrap();
    assert_eq!(
        first.as_object().unwrap().keys().collect::<Vec<_>>(),
        second.as_object().unwrap().keys().collect::<Vec<_>>()
    );
    assert_eq!(second["session_id"], "ses_created_2");
    let prompted = fixture.run(&[
        "prompt-session",
        "ses_created_1",
        "Follow up",
        "--origin",
        "slack://message/3",
        "--json",
    ]);
    assert!(
        prompted.status.success(),
        "{}",
        String::from_utf8_lossy(&prompted.stderr)
    );
    let listing = fixture.run(&["list-sessions", "--json", "--include-closed"]);
    assert!(
        listing.status.success(),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    let listing: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    let sessions = listing["sessions"].as_array().unwrap();
    assert_eq!(
        sessions
            .iter()
            .find(|session| session["session_id"] == "ses_created_1")
            .unwrap()["origins"],
        json!(["slack://message/1", "slack://message/3"])
    );
    assert_eq!(
        sessions
            .iter()
            .find(|session| session["session_id"] == "ses_created_2")
            .unwrap()["origins"],
        json!(["slack://message/2"])
    );
}

#[test]
fn instance_startup_failures_preserve_json_receipts_for_submitted_input() {
    let fixture = Fixture::new();
    let _companion = Companion::start(&fixture);
    let output = wait_output(
        fixture
            .command(&[
                "new-instance",
                "review",
                "-t",
                "website",
                "--opencode",
                "Initial task",
                "--origin",
                "correlation",
                "--json",
            ])
            .env("FAIL_START", "1")
            .spawn()
            .unwrap(),
    );
    assert!(!output.status.success());
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["session_id"], "ses_created_1");
    assert_eq!(receipt["prompt_outcome"], "submitted");
    assert!(String::from_utf8_lossy(&output.stderr).contains("fixture startup failed"));
}

#[test]
fn blank_instance_sessions_return_identity_without_submitting_input() {
    let fixture = Fixture::new();
    let _companion = Companion::start(&fixture);
    let output = fixture.run(&[
        "new-instance",
        "review",
        "-t",
        "website",
        "--opencode",
        "--json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["session_id"], "ses_created_1");
    assert_eq!(receipt["prompt_outcome"], "not_submitted");
}
