use super::*;

#[test]
fn provider_stop_verifies_its_collector_without_scanning_other_providers() {
    let fixture = Fixture::new();
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0007_providers.sql"))
        .unwrap();
    for name in ["message", "ticket", "system-event", "generic"] {
        let directory = fixture.home.join("templates/providers").join(name);
        fs::create_dir_all(&directory).unwrap();
        let manifest = json!({
            "schema_version": 1, "name": format!("dev-{name}"), "profile": "generic",
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

const PROVIDER_DOCKER: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$TANDEM_HOME/provider-commands"
case "$1" in
    ps)
        case "$*" in *cli-test-provider-message*) printf '%s\n' fixture-provider ;; esac
        ;;
    inspect)
        if [ "$2" = fixture-provider ]; then
            running=true
            if [ -f "$TANDEM_HOME/provider-stopped" ]; then running=false; fi
            printf '[{"Id":"fixture-provider","Config":{"Labels":{"io.tandem.provider-namespace":"cli-test","io.tandem.provider-name":"message","com.docker.compose.project":"cli-test-provider-message","com.docker.compose.service":"collector"}},"State":{"Running":%s,"Paused":false}}]\n' "$running"
        else
            printf '[]\n'
        fi
        ;;
    stop)
        : > "$TANDEM_HOME/provider-stopped"
        printf '%s\n' fixture-provider
        ;;
esac
"#;
