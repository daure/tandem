use std::{
    io::{BufRead, BufReader, Write},
    process::ChildStdin,
    sync::mpsc,
};

use serde_json::Value;

use super::*;

#[cfg(target_os = "linux")]
#[path = "startup_tui.rs"]
mod tui;

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    replies: mpsc::Receiver<Value>,
    next_id: u64,
}

impl Client {
    fn new(fixture: &Fixture) -> Self {
        Self::with_command(fixture.command(&["mcp"]))
    }

    fn with_command(mut command: Command) -> Self {
        use std::os::unix::process::CommandExt;
        let mut child = command
            .env("BLOCK_CONFIG", "1")
            .stdin(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (sender, replies) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Ok(message) = serde_json::from_str(&line)
                    && sender.send(message).is_err()
                {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            replies,
            next_id: 1,
        };
        client.request("initialize", json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "startup-test", "version": "1"}}));
        client.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        client
    }

    fn send(&mut self, value: Value) {
        writeln!(self.input.as_mut().unwrap(), "{value}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let response = self
                .replies
                .recv_timeout(Duration::from_secs(15))
                .expect("MCP response timed out");
            if response["id"] == id {
                assert!(response.get("error").is_none(), "{response}");
                return response["result"].clone();
            }
        }
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        assert_ne!(result["isError"], true, "{result}");
        if let Some(value) = result.get("structuredContent") {
            return value.clone();
        }
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn close(&mut self, signal: Option<i32>) {
        if let Some(signal) = signal {
            assert_eq!(unsafe { libc::kill(-(self.child.id() as i32), signal) }, 0);
        } else {
            self.input.take();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "client shutdown waited for startup"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "startup condition timed out");
        thread::sleep(Duration::from_millis(30));
    }
}

fn start(client: &mut Client) -> Value {
    client.tool("create_instance", json!({"template": "website", "name": "review", "confirmed": true, "wait": false, "timeout_seconds": 30}))
}

#[cfg(target_os = "linux")]
#[test]
fn startup_uses_the_running_binary_after_its_executable_is_replaced() {
    let fixture = Fixture::new();
    let executable = fixture.bin.join("tandem");
    fs::copy(env!("CARGO_BIN_EXE_tandem"), &executable).unwrap();
    let mut client = Client::with_command(fixture.command_at(&executable, &["mcp"]));
    let replacement = fixture.bin.join("replacement");
    fs::write(&replacement, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&replacement, &executable).unwrap();
    assert_eq!(
        fs::read_link(format!("/proc/{}/exe", client.child.id())).unwrap(),
        PathBuf::from(format!("{} (deleted)", executable.display()))
    );

    fs::write(fixture.home.join("release-config"), "").unwrap();
    let operation = start(&mut client);
    assert_eq!(operation["state"], "running", "{operation}");
    wait_until(|| {
        client.tool("get_operation", json!({"id": operation["id"]}))["state"] != "running"
    });
    let completed = client.tool("get_operation", json!({"id": operation["id"]}));
    assert_eq!(completed["state"], "succeeded", "{completed}");
    assert!(
        fixture
            .home
            .join("workspaces/review/app/file.txt")
            .is_file()
    );
    assert!(fixture.home.join("workspaces/review/AGENTS.md").is_file());
}

#[test]
fn startup_survives_client_exit_hangup_and_kill_and_reconnects_without_reexecution() {
    for signal in [None, Some(libc::SIGHUP), Some(libc::SIGKILL)] {
        let fixture = Fixture::new();
        let mut client = Client::new(&fixture);
        let operation = start(&mut client);
        assert_eq!(operation["state"], "running", "{operation}");
        wait_until(|| fixture.home.join("configured").exists());
        client.close(signal);

        let mut reopened = Client::new(&fixture);
        let running = reopened.tool("get_operation", json!({"id": operation["id"]}));
        assert_eq!(running["state"], "running", "{running}");
        let inventory = reopened.tool("list_instances", json!({}));
        assert_eq!(inventory["instances"][0]["name"], "review", "{inventory}");
        assert_eq!(inventory["instances"][0]["pending"], true);
        let duplicate = reopened.request("tools/call", json!({"name": "create_instance", "arguments": {"template": "website", "name": "review", "confirmed": true, "wait": false}}));
        assert!(
            duplicate["isError"] == true || duplicate.to_string().contains("failed"),
            "{duplicate}"
        );
        assert_eq!(
            reopened.tool("get_operation", json!({"id": operation["id"]}))["state"],
            "running"
        );

        fs::write(fixture.home.join("release-config"), "").unwrap();
        wait_until(|| {
            reopened.tool("get_operation", json!({"id": operation["id"]}))["state"] != "running"
        });
        let completed = reopened.tool("get_operation", json!({"id": operation["id"]}));
        assert_eq!(completed["state"], "succeeded", "{completed}");
        assert!(fixture.home.join("workspaces/review/app/file.txt").exists());
        assert_eq!(
            fs::read_to_string(fixture.home.join("docker-calls"))
                .unwrap()
                .lines()
                .filter(|line| line.contains("cli-test-review") && line.contains("up --detach"))
                .count(),
            1
        );
    }
}

#[test]
fn requested_client_launch_and_cloning_survive_the_initiating_cli_exit() {
    let fixture = Fixture::new();
    let mut client = fixture
        .command(&["new-instance", "review", "-t", "website", "-o"])
        .env("BLOCK_OPEN", "1")
        .spawn()
        .unwrap();
    wait_until(|| fixture.home.join("opening").exists());
    let workspace = fixture.home.join("workspaces/review");
    assert!(workspace.is_dir());
    assert!(!workspace.join("app").exists());
    client.kill().unwrap();
    client.wait().unwrap();
    fs::write(fixture.home.join("release-open"), "").unwrap();
    let record_path = fixture.home.join("runtime/cli-test/review.startup.json");
    wait_until(|| {
        let record: Value =
            serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
        record["operation"]["state"] != "running"
    });
    let record: Value = serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(record["operation"]["state"], "succeeded", "{record}");
    assert_eq!(record["opencode_result"], json!({"Ok": null}));
    assert!(workspace.join("app/file.txt").is_file());
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened")).unwrap(),
        format!("{}\n", workspace.display())
    );
}

#[test]
fn worker_death_is_reported_as_interrupted_after_reconnecting() {
    let fixture = Fixture::new();
    let mut client = Client::new(&fixture);
    let operation = start(&mut client);
    wait_until(|| fixture.home.join("configured").exists());
    let record: Value = serde_json::from_str(
        &fs::read_to_string(fixture.home.join("runtime/cli-test/review.startup.json")).unwrap(),
    )
    .unwrap();
    let pid = record["owner_pid"].as_i64().unwrap() as i32;
    assert!(pid > 0);
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    client.close(None);
    let mut reopened = Client::new(&fixture);
    wait_until(|| {
        reopened.tool("get_operation", json!({"id": operation["id"]}))["state"] == "failed"
    });
    let failed = reopened.tool("get_operation", json!({"id": operation["id"]}));
    assert!(failed["error"].as_str().unwrap().contains("interrupted"));
    // The child command cannot retain the worker's lease while waiting on dependencies.
    fs::write(fixture.home.join("release-config"), "").unwrap();
    let inventory = reopened.tool("list_instances", json!({}));
    assert_eq!(inventory["instances"][0]["pending"], false);
    assert!(
        inventory["instances"][0]["runtime"]["issue"]
            .as_str()
            .unwrap()
            .contains("interrupted")
    );
}
