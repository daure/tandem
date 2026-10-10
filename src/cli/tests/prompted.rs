use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn prompted_sessions_preserve_existing_work_and_return_delivery_receipts_without_resending() {
    for (fresh_client, result) in [
        (false, "submitted"),
        (true, "submitted"),
        (false, "uncertain"),
        (false, "disconnected"),
        (false, "wrong-server"),
    ] {
        let fixture = Fixture::new();
        let created = fixture.run(&["new-instance", "review", "-t", "website"]);
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let workspace = fixture.home.join("workspaces/review");
        fs::write(workspace.join("app/file.txt"), "Uncommitted work").unwrap();
        fs::write(workspace.join("AGENTS.md"), "User guidance\n").unwrap();
        let startup = fixture.runtime_record("review", "startup");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let receipt = json!({
            "pid":std::process::id(),
            "observed_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            "id":"", "title":"OpenCode", "directory":workspace,
            "server":"http://127.0.0.1:4199", "activity":"idle",
            "zellij_session":"main", "pane_id":100,
            "tab_control": {
                "server":format!("http://{}", listener.local_addr().unwrap()),
                "token":"client-secret", "prompted_sessions":true, "session_tabs":true,
            },
        });
        let presence = fixture.home.join("state/tandem/opencode");
        fs::create_dir_all(&presence).unwrap();
        if fresh_client {
            fs::write(
                fixture.home.join("client-receipt.json"),
                receipt.to_string(),
            )
            .unwrap();
            fs::write(fixture.bin.join("opencode"), "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '1.18.29\\n'; exit 0; fi\ncp \"$TANDEM_HOME/client-receipt.json\" \"$XDG_STATE_HOME/tandem/opencode/client.json\"\n").unwrap();
        } else {
            fs::write(presence.join("client.json"), receipt.to_string()).unwrap();
        }
        let home = fixture.home.clone();
        let controller = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let (mut stream, _) = loop {
                if let Ok(client) = listener.accept() {
                    break client;
                }
                assert!(Instant::now() < deadline, "missing session request");
                thread::sleep(Duration::from_millis(5));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "POST /sessions HTTP/1.1\r\n");
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
                if line == "authorization: Bearer client-secret\r\n" {
                    authorized = true;
                }
            }
            assert!(authorized);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(home.join("locks/instance-review"))
                .unwrap();
            assert!(
                lock.try_lock().is_err(),
                "session opening must hold the instance lock"
            );
            if result != "disconnected" {
                let response = json!({
                    "session_id":"ses_new", "server":if result == "wrong-server" { "http://127.0.0.1:4299" } else { "http://127.0.0.1:4199" },
                    "prompt_outcome":if result == "uncertain" { "uncertain" } else { "submitted" },
                    "error":if result == "uncertain" { Some("Prompt delivery may be uncertain") } else { None },
                }).to_string();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).unwrap();
            }
            body
        });
        let prompt = "Inspect 'this'; $(touch injected)\n日本語";
        let output = fixture.run(&[
            "new-instance-session",
            "review",
            prompt,
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
            result == "submitted",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let sent = controller.join().unwrap();
        assert_eq!(sent["directory"], workspace.to_str().unwrap());
        assert_eq!(sent["initial_prompt"], prompt);
        assert_eq!(sent["model"], "openai/test#high");
        assert_eq!(sent["variant"], "high");
        assert_eq!(sent["agent"], "tracer");
        assert!(
            sent["instructions"]
                .as_str()
                .unwrap()
                .contains("Services are started")
        );
        let outcome: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            outcome["prompt_outcome"],
            if result == "submitted" {
                "submitted"
            } else {
                "uncertain"
            }
        );
        assert_eq!(
            outcome["session_id"],
            if matches!(result, "disconnected" | "wrong-server") {
                serde_json::Value::Null
            } else {
                json!("ses_new")
            }
        );
        assert_eq!(outcome["server"], "http://127.0.0.1:4199");
        assert_eq!(
            fs::read_to_string(workspace.join("app/file.txt")).unwrap(),
            "Uncommitted work"
        );
        assert_eq!(
            fs::read_to_string(workspace.join("AGENTS.md")).unwrap(),
            "User guidance\n"
        );
        assert_eq!(fixture.runtime_record("review", "startup"), startup);
        let calls = fs::read_to_string(fixture.home.join("zellij-calls")).unwrap();
        assert_eq!(calls.matches("new-tab").count(), usize::from(fresh_client));
        assert!(!calls.contains("new-pane") && !calls.contains("close-pane"));
        assert!(!workspace.join("injected").exists());
    }
}

#[test]
fn prompted_sessions_reject_unowned_and_busy_instances_before_contacting_opencode() {
    for blocked in ["missing", "busy", "ownership"] {
        let fixture = Fixture::new();
        let lock = if blocked != "missing" {
            assert!(
                fixture
                    .run(&["new-instance", "review", "-t", "website"])
                    .status
                    .success()
            );
            if blocked == "ownership" {
                fixture.remove_runtime_record("review", "ownership");
                None
            } else {
                let lock = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(fixture.home.join("locks/instance-review"))
                    .unwrap();
                lock.lock().unwrap();
                Some(lock)
            }
        } else {
            None
        };
        let output = fixture.run(&["new-instance-session", "review", "Inspect this"]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(if blocked == "busy" {
                "busy"
            } else {
                "ownership"
            }),
            "{error}"
        );
        assert!(!fixture.home.join("zellij-calls").exists());
        drop(lock);
    }
}
