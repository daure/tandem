use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

struct Server {
    url: String,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(home: &Path, directory: &Path, result: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let home = home.to_owned();
        let directory = directory.to_owned();
        let stop = Arc::new(AtomicBool::new(false));
        let received = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::clone(&stop);
        let inputs = Arc::clone(&received);
        let server = url.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let method = line.split_whitespace().next().unwrap().to_owned();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0;
                let mut authorized = false;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("content-length: ") {
                        length = value.trim().parse().unwrap();
                    }
                    authorized |= line == "authorization: Bearer client-secret\r\n";
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                if result.starts_with("listing") {
                    assert_eq!(method, "GET");
                }
                if result == "listing-unavailable" && path != "/api/info" {
                    stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    continue;
                }
                let remote = |id: &str, location: &Path| {
                    json!({
                        "id":id, "title":format!("Title {id}"), "location":{"directory":location},
                        "time":{"created":1,"updated":2},
                    })
                };
                let response = match path.as_str() {
                    "/api/info" => json!({"version":"2.0.22"}),
                    "/api/session/ses_existing" => json!({"data":remote("ses_existing", &directory)}),
                    "/api/session/ses_external" => json!({"data":remote("ses_external", directory.parent().unwrap())}),
                    "/api/session/active" => json!({"data":{"ses_existing":{}}}),
                    path if path.starts_with("/api/form?") || path.starts_with("/api/permission/request?") => json!({"data":[]}),
                    path if path.starts_with("/api/session?") => json!({"data":[
                        remote("ses_existing", &directory),
                        remote("ses_saved", &directory.join("app")),
                        remote("ses_external", directory.parent().unwrap()),
                    ]}),
                    path if path.starts_with("/api/experimental/session/") && path.ends_with("/export") => json!({"data":{"messages":[]}}),
                    "/sessions/prompt" => {
                        assert!(authorized);
                        let lock = fs::OpenOptions::new().read(true).write(true)
                            .open(home.join("locks/instance-review")).unwrap();
                        assert!(lock.try_lock().is_err());
                        inputs.lock().unwrap().push(serde_json::from_slice(&bytes).unwrap());
                        if result == "disconnected" { continue; }
                        json!({"session_id":if result == "wrong-id" { "ses_other" } else { "ses_existing" },
                            "server":server, "prompt_outcome":"queued", "error":null})
                    }
                    _ => panic!("unexpected path: {path}"),
                }.to_string();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).unwrap();
            }
        });
        Self {
            url,
            received,
            stop,
            thread: Some(thread),
        }
    }

    fn presence(&self, fixture: &Fixture, directory: &Path, file: &str) -> serde_json::Value {
        let value = json!({
            "pid":std::process::id(),
            "observed_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            "id":"ses_existing", "title":"OpenCode", "directory":directory, "server":self.url,
            "activity":"idle", "zellij_session":"main", "pane_id":100,
            "tab_control":{"server":self.url,"token":"client-secret", "prompted_sessions":true,
                "session_prompts":true,"session_tabs":true},
        });
        let root = fixture.home.join("state/tandem/opencode");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(file), value.to_string()).unwrap();
        value
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn prompts_resolve_owned_sessions_reopen_exact_history_and_never_resend_uncertain_input() {
    for result in ["queued", "detached", "disconnected", "wrong-id"] {
        let fixture = Fixture::new();
        assert!(
            fixture
                .run(&["new-instance", "review", "-t", "website"])
                .status
                .success()
        );
        let directory = fixture.home.join("workspaces/review");
        fs::write(directory.join("app/file.txt"), "Uncommitted work").unwrap();
        let startup = fixture.runtime_record("review", "startup");
        let server = Server::start(&fixture.home, &directory, result);
        let receipt = server.presence(&fixture, &directory, "client.json");
        if result == "detached" {
            fs::remove_file(fixture.home.join("state/tandem/opencode/client.json")).unwrap();
            let daemon = fixture.home.join("daemons/fixture");
            fs::create_dir_all(daemon.join("dirs")).unwrap();
            fs::write(daemon.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
            fs::write(
                daemon.join("dirs/workspace.dir"),
                directory.to_str().unwrap(),
            )
            .unwrap();
            fs::write(
                fixture.home.join("client-receipt.json"),
                receipt.to_string(),
            )
            .unwrap();
            fs::write(fixture.bin.join("opencode-station"), "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$TANDEM_HOME/station-calls\"\ncp \"$TANDEM_HOME/client-receipt.json\" \"$XDG_STATE_HOME/tandem/opencode/client.json\"\n").unwrap();
            fs::set_permissions(
                fixture.bin.join("opencode-station"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let output = fixture.run(&[
            "prompt-session",
            "ses_existing",
            "Literal 'text'; $(touch injected)",
            "--model",
            "openai/test",
            "--variant",
            "high",
            "--agent",
            "tracer",
            "--json",
        ]);
        assert_eq!(
            output.status.success(),
            matches!(result, "queued" | "detached"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let outcome: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(outcome["session_id"], "ses_existing");
        assert_eq!(outcome["server"], server.url);
        assert_eq!(
            outcome["prompt_outcome"],
            if matches!(result, "queued" | "detached") {
                "queued"
            } else {
                "uncertain"
            }
        );
        let inputs = server.received.lock().unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0]["sessionID"], "ses_existing");
        assert_eq!(inputs[0]["when_busy"], "queue");
        assert_eq!(inputs[0]["agent"], "tracer");
        assert_eq!(inputs[0]["model"], "openai/test#high");
        assert_eq!(
            inputs[0]["initial_prompt"],
            "Literal 'text'; $(touch injected)"
        );
        let calls = fs::read_to_string(fixture.home.join("zellij-calls")).unwrap();
        assert_eq!(
            calls.matches("new-tab").count(),
            usize::from(result == "detached")
        );
        if result == "detached" {
            let calls = fs::read_to_string(fixture.home.join("station-calls")).unwrap();
            assert!(calls.contains("--session ses_existing"));
            assert!(!calls.contains("run"));
        }
        assert_eq!(
            fs::read_to_string(directory.join("app/file.txt")).unwrap(),
            "Uncommitted work"
        );
        assert_eq!(fixture.runtime_record("review", "startup"), startup);
        assert!(!directory.join("injected").exists());
    }
}

#[test]
fn session_prompts_reject_external_ambiguous_and_unowned_targets_before_sending_input() {
    for blocked in ["external", "ambiguous", "ownership"] {
        let fixture = Fixture::new();
        assert!(
            fixture
                .run(&["new-instance", "review", "-t", "website"])
                .status
                .success()
        );
        let directory = if blocked == "external" {
            fixture.source.clone()
        } else {
            fixture.home.join("workspaces/review")
        };
        let server = Server::start(&fixture.home, &directory, "queued");
        server.presence(&fixture, &directory, "client.json");
        let second = if blocked == "ambiguous" {
            let other = Server::start(&fixture.home, &directory, "queued");
            other.presence(&fixture, &directory, "second.json");
            Some(other)
        } else {
            None
        };
        if blocked == "ownership" {
            fixture.remove_runtime_record("review", "ownership");
        }
        let output = fixture.run(&["prompt-session", "ses_existing", "Inspect this"]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(match blocked {
                "external" => "outside an owned",
                "ambiguous" => "ambiguous",
                _ => "ownership",
            }),
            "{error}"
        );
        assert!(server.received.lock().unwrap().is_empty());
        assert!(!fixture.home.join("zellij-calls").exists());
        drop(second);
    }
}

#[test]
fn session_listing_discovers_instance_ids_history_and_external_sessions_without_mutation() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .run(&["new-instance", "review", "-t", "website"])
            .status
            .success()
    );
    let directory = fixture.home.join("workspaces/review");
    let startup = fixture.runtime_record("review", "startup");
    let server = Server::start(&fixture.home, &directory, "listing");
    let mut external = server.presence(&fixture, &directory, "client.json");
    external["id"] = json!("ses_external");
    external["directory"] = json!(directory.parent().unwrap());
    external["pane_id"] = json!(null);
    rusqlite::Connection::open(fixture.home.join("settings.sqlite3"))
        .unwrap()
        .execute_batch("ALTER TABLE event_rules DROP COLUMN catalog_present;")
        .unwrap();
    let rule = fixture.home.join("templates/rules/unreadable/rule.json");
    fs::create_dir_all(rule.parent().unwrap()).unwrap();
    fs::write(&rule, "{").unwrap();
    fs::write(
        fixture.home.join("state/tandem/opencode/external.json"),
        external.to_string(),
    )
    .unwrap();
    let output = fixture.run(&["list-sessions", "--instance", "review", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let sessions = listing["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "{listing}");
    assert_eq!(sessions[0]["session_id"], "ses_existing");
    assert_eq!(sessions[0]["instance"], "review");
    assert_eq!(sessions[0]["activity"], "busy");
    assert_eq!(sessions[0]["attached"], true);
    assert_eq!(sessions[0]["availability"], "observed");
    assert_eq!(sessions[0]["server"], server.url);
    assert_eq!(sessions[0]["panes"][0]["id"], 100);
    let output = fixture.run(&[
        "list-sessions",
        "--instance",
        "review",
        "--include-closed",
        "--json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let sessions = listing["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2, "{listing}");
    assert_eq!(sessions[1]["session_id"], "ses_saved");
    assert_eq!(
        sessions[1]["directory"],
        directory.join("app").to_str().unwrap()
    );
    assert_eq!(sessions[1]["activity"], "idle");
    assert_eq!(sessions[1]["attached"], false);
    assert_eq!(listing["history_window_per_directory"], 21);
    assert!(listing["observation_error"].is_null());
    let output = fixture.run(&["list-sessions", "--include-closed", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let external = listing["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session_id"] == "ses_external")
        .unwrap();
    assert!(external["instance"].is_null());
    let output = fixture.run(&["list-sessions", "--instance", "review"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("ses_existing")
            && !text.contains("ses_saved")
            && !text.contains("ses_external"),
        "{text}"
    );
    assert!(server.received.lock().unwrap().is_empty());
    let calls = fs::read_to_string(fixture.home.join("zellij-calls")).unwrap();
    assert!(
        !calls.contains("new-tab") && !calls.contains("focus-pane-id"),
        "{calls}"
    );
    assert_eq!(fixture.runtime_record("review", "startup"), startup);
    assert_eq!(fs::read_to_string(rule).unwrap(), "{");
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    let rules: i64 = connection
        .query_row("SELECT count(*) FROM event_rules", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rules, 0);
    let migrated: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('event_rules') WHERE name = 'catalog_present')", [], |row| row.get(0)).unwrap();
    assert!(migrated);
}

#[test]
fn session_listing_reports_partial_observation_and_rejects_disabled_or_unknown_targets() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .run(&["new-instance", "review", "-t", "website"])
            .status
            .success()
    );
    let directory = fixture.home.join("workspaces/review");
    let server = Server::start(&fixture.home, &directory, "listing-unavailable");
    server.presence(&fixture, &directory, "client.json");
    let output = fixture.run(&["list-sessions", "--instance", "review", "--json"]);
    assert!(!output.status.success());
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listing["sessions"][0]["session_id"], "ses_existing");
    assert_eq!(listing["sessions"][0]["stale"], true);
    assert_eq!(listing["sessions"][0]["availability"], "unverified");
    assert_eq!(listing["sessions"][0]["activity"], "unknown");
    assert!(
        listing["observation_error"]
            .as_str()
            .unwrap()
            .contains("observation unavailable")
    );
    let output = fixture.run(&["list-sessions", "--instance", "absent", "--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("instance absent is unavailable"));
    rusqlite::Connection::open(fixture.home.join("settings.sqlite3"))
        .unwrap()
        .execute(
            "INSERT INTO app_settings(key, value) VALUES ('integrations.opencode', 'false')",
            [],
        )
        .unwrap();
    let output = fixture.run(&["list-sessions", "--json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("integration is disabled"));
    assert!(server.received.lock().unwrap().is_empty());
}
