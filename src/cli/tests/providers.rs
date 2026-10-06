use super::*;

#[path = "provider_streams.rs"]
mod streams;

#[path = "provider_deletion.rs"]
mod deletion;

#[cfg(target_os = "linux")]
#[path = "sidecar.rs"]
mod sidecar;

fn provider_fixture() -> Fixture {
    let fixture = Fixture::new();
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0007_providers.sql"))
        .unwrap();
    for name in ["message", "ticket", "system-event", "generic"] {
        let directory = fixture.home.join("templates/providers").join(name);
        fs::create_dir_all(&directory).unwrap();
        let manifest = json!({
            "schema_version": 2, "name": format!("dev-{name}"),
            "description": "Provider fixture", "protocol": "tandem-events-v1"
        });
        fs::write(directory.join("Dockerfile"), "FROM scratch\n").unwrap();
        fs::write(directory.join("provider.json"), manifest.to_string()).unwrap();
        connection
            .execute(
                "INSERT INTO provider_launches(namespace, name, directory, manifest, compose)
             VALUES ('cli-test', ?1, ?2, ?3, '{}')",
                rusqlite::params![name, directory.to_string_lossy(), manifest.to_string()],
            )
            .unwrap();
    }
    fs::write(fixture.bin.join("docker"), PROVIDER_DOCKER).unwrap();
    fixture
}

#[test]
fn provider_stop_verifies_its_collector_without_scanning_other_providers() {
    let fixture = provider_fixture();
    let output = fixture.run(&["provider", "stop", "message"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Provider message: Stop"));
    let commands = fs::read_to_string(fixture.home.join("provider-commands")).unwrap();
    let commands: Vec<_> = commands
        .lines()
        .filter(|line| line.contains("-provider-") || line.contains("fixture-provider"))
        .collect();
    assert_eq!(commands.len(), 4, "{commands:#?}");
    assert!(commands[0].starts_with("ps "), "{commands:#?}");
    assert_eq!(commands[1], "inspect fixture-provider");
    assert!(commands[2].starts_with("stop "), "{commands:#?}");
    assert_eq!(commands[3], "inspect fixture-provider");
    assert!(
        commands.iter().all(|line| {
            !line.contains("provider-ticket")
                && !line.contains("provider-system-event")
                && !line.contains("provider-generic")
        }),
        "{commands:#?}"
    );
}

struct Sidecar(Child);

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn provider_start_and_stop_recover_an_owned_paused_collector() {
    let fixture = provider_fixture();
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
    fs::remove_dir_all(fixture.home.join("templates/providers/message")).unwrap();
    for (action, status) in [("start", "running"), ("stop", "stopped")] {
        fs::write(fixture.home.join("provider-paused"), "").unwrap();
        let output = fixture.run(&["provider", action, "message"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = fixture.run(&["list-providers"]);
        assert!(output.status.success());
        let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let provider = snapshot["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|provider| provider["name"] == "message")
            .unwrap();
        assert_eq!(provider["status"], status);
        assert_eq!(provider["container_id"], "fixture-provider");
        assert!(!fixture.home.join("provider-paused").exists());
    }
    let commands = fs::read_to_string(fixture.home.join("provider-commands")).unwrap();
    assert_eq!(
        commands
            .lines()
            .filter(|line| *line == "unpause fixture-provider")
            .count(),
        2
    );
}

const PROVIDER_DOCKER: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$TANDEM_HOME/provider-commands"
case "$1" in
    ps)
        if [ -f "$TANDEM_HOME/provider-absent" ]; then exit 0; fi
        case "$*" in *cli-test-provider-message*) printf '%s\n' fixture-provider ;; esac
        ;;
    inspect)
        if [ "$2" = fixture-provider ]; then
            running=true
            if [ -f "$TANDEM_HOME/provider-stopped" ]; then running=false; fi
            paused=false
            if [ -f "$TANDEM_HOME/provider-paused" ]; then paused=true; fi
            printf '[{"Id":"fixture-provider","Config":{"Labels":{"io.tandem.provider-namespace":"cli-test","io.tandem.provider-name":"message","com.docker.compose.project":"cli-test-provider-message","com.docker.compose.service":"collector"}},"State":{"Running":%s,"Paused":%s}}]\n' "$running" "$paused"
        else
            printf '[]\n'
        fi
        ;;
    stop)
        : > "$TANDEM_HOME/provider-stopped"
        printf '%s\n' fixture-provider
        ;;
    unpause)
        rm "$TANDEM_HOME/provider-paused"
        printf '%s\n' fixture-provider
        ;;
    compose)
        rm -f "$TANDEM_HOME/provider-stopped" "$TANDEM_HOME/provider-absent"
        ;;
esac
"#;
