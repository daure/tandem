use super::*;
use std::{
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
};

fn connect(path: &Path) -> BufReader<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(stream) = UnixStream::connect(path) {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            return BufReader::new(stream);
        }
        assert!(Instant::now() < deadline, "state listener did not start");
        thread::sleep(Duration::from_millis(10));
    }
}

fn next(reader: &mut BufReader<UnixStream>) -> serde_json::Value {
    let mut line = String::new();
    assert!(reader.read_line(&mut line).unwrap() > 0);
    serde_json::from_str(&line).unwrap()
}

struct Server(Child);

impl Server {
    fn start(fixture: &Fixture) -> Self {
        Self(
            fixture
                .command(&["serve-state"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn headless_state_tracks_cli_changes_and_subscribers_do_not_trigger_collection() {
    let fixture = Fixture::new();
    let output = fixture.run(&["state-socket"]);
    assert!(output.status.success());
    let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let server = Server::start(&fixture);
    let mut subscriber = connect(&path);
    assert_eq!(next(&mut subscriber)["type"], "snapshot");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.home.join("docker-calls").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let before = fs::read_to_string(fixture.home.join("docker-calls")).unwrap();
    let mut subscribers = Vec::new();
    for _ in 0..15 {
        let mut reader = connect(&path);
        assert_eq!(next(&mut reader)["type"], "snapshot");
        subscribers.push(reader);
    }
    assert_eq!(
        fs::read_to_string(fixture.home.join("docker-calls")).unwrap(),
        before
    );
    assert!(!before.lines().any(|line| line.starts_with("stats")));
    let created = fixture.run(&["new-instance", "review", "-t", "website"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    loop {
        let event = next(&mut subscriber);
        if event["type"] == "instance.changed" && event["data"]["name"] == "review" {
            assert!(event["data"]["summary"].is_object());
            assert!(event["data"]["workspace_only"].is_boolean());
            break;
        }
        assert!(Instant::now() < deadline + Duration::from_secs(5));
    }
    drop(subscribers);
    drop(subscriber);
    drop(server);
    let server = Server::start(&fixture);
    let mut reconnected = connect(&path);
    let snapshot = next(&mut reconnected);
    assert_eq!(snapshot["type"], "snapshot");
    assert!(
        snapshot["data"]["instances"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .any(|instance| instance["name"] == "review")
    );
    drop(server);
}

#[test]
fn restart_with_failed_docker_observation_preserves_stale_instances() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .run(&["new-instance", "review", "-t", "website"])
            .status
            .success()
    );
    let first = Server::start(&fixture);
    let output = fixture.run(&["state-socket"]);
    assert!(output.status.success());
    let socket = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let mut reader = connect(&socket);
    loop {
        let event = next(&mut reader);
        if event["type"] == "instance.changed" && event["data"]["name"] == "review"
            || event["type"] == "snapshot"
                && event["data"]["instances"]["instances"]
                    .as_array()
                    .is_some_and(|instances| {
                        instances
                            .iter()
                            .any(|instance| instance["name"] == "review")
                    })
        {
            break;
        }
    }
    drop(reader);
    drop(first);
    fs::write(
        fixture.bin.join("docker"),
        "#!/bin/sh\nprintf 'Docker unavailable in fixture\\n' >&2\nexit 1\n",
    )
    .unwrap();
    let _restarted = Server::start(&fixture);
    let mut reader = connect(&socket);
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    loop {
        let event = next(&mut reader);
        assert_ne!(event["type"], "instance.removed");
        if matches!(event["type"].as_str(), Some("state.changed" | "snapshot"))
            && !event["data"]["instances"]["runtime_error"].is_null()
        {
            break;
        }
    }
    let snapshot = next(&mut connect(&socket));
    let inventory = &snapshot["data"]["instances"];
    assert!(
        inventory["runtime_error"]
            .as_str()
            .unwrap()
            .contains("Docker unavailable")
    );
    let instance = inventory["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| instance["name"] == "review")
        .unwrap();
    assert_eq!(instance["runtime"]["stale"], true);
    assert_eq!(instance["services"][0]["runtime"]["stale"], true);
}
