use super::super::startup::Client;
use super::*;

use super::super::history::history_server;

fn fixture() -> (Fixture, history_server::Server) {
    let fixture = provider_fixture();
    fs::create_dir(fixture.home.join("templates/blank")).unwrap();
    fs::write(fixture.home.join("templates/blank/tandem.json"), "{}").unwrap();
    fs::write(fixture.bin.join("docker"), DOCKER).unwrap();
    let created = fixture.run(&["new-instance", "review", "-t", "blank"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let startup = fixture.runtime_record("review", "startup").unwrap();
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    let rule = json!({"definition":{"name":"matching","description":"Matching events","script":"true","template":"blank","model":"test/model","initial_prompt":"Inspect","enabled":false,"start_instance":true},"revision":1,"zellij_session":"main"});
    let event = json!({"schema_version":1,"event_id":"event-1","stream":"messages","type":"test.created","summary":"Test event","profile":"generic","data":{}});
    for (namespace, source, sequence) in [
        ("cli-test", "dev-message", 1),
        ("cli-test", "other", 2),
        ("other-test", "dev-message", 3),
    ] {
        connection
            .execute(
                "INSERT INTO event_providers(namespace,name,token) VALUES (?1,?2,?3)",
                rusqlite::params![namespace, source, format!("{namespace}-{source}")],
            )
            .unwrap();
        connection.execute("INSERT INTO events(sequence,namespace,provider,event_id,received_at,payload) VALUES (?1,?2,?3,'event-1','now',?4)", rusqlite::params![sequence, namespace, source, event.to_string()]).unwrap();
        connection.execute("INSERT INTO event_attempts(id,event_sequence,status,replay,created_at) VALUES (?1,?1,'accepted',0,'now')", [sequence]).unwrap();
    }
    let acceptance = json!({"id":1,"event_sequence":1,"event_summary":"Test event","attempt_id":1,"rule_name":"matching","rule_revision":1,"accepted_at":"now","instance":"review","session_id":"ses_provider","pane":null,"operation_id":startup["operation"]["id"],"launch_started_at":"now","status":"launched","error":null,"rule":rule,"resolved_prompt":"Inspect"});
    connection.execute("INSERT INTO rule_acceptances(id,attempt_id,rule_name,payload) VALUES (1,1,'matching',?1)", [acceptance.to_string()]).unwrap();
    connection.execute("INSERT INTO rule_evaluations(attempt_id,rule_name,rule_revision,rule_snapshot) VALUES (1,'matching',1,?1)", [rule.to_string()]).unwrap();
    connection.execute("INSERT INTO provider_notifications(namespace,provider,payload) VALUES ('cli-test','dev-message','{}')", []).unwrap();
    connection.execute("INSERT INTO provider_streams(namespace,provider,stream) VALUES ('cli-test','dev-message','messages')", []).unwrap();
    let runtime = fixture.home.join("runtime/cli-test/providers/message");
    fs::create_dir_all(&runtime).unwrap();
    fs::write(runtime.join("compose.json"), "{}").unwrap();
    let credentials = fixture.home.join("provider-credentials/cli-test");
    fs::create_dir_all(&credentials).unwrap();
    fs::write(credentials.join("dev-message.token"), "token").unwrap();
    let server = history_server::Server::start();
    let workspace = fixture.home.join("workspaces/review");
    server.session("ses_provider", workspace.to_str().unwrap(), None);
    server.session("ses_unrelated", fixture.source.to_str().unwrap(), None);
    let daemon = fixture.home.join("daemons/test");
    fs::create_dir_all(daemon.join("dirs")).unwrap();
    fs::write(daemon.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    fs::write(daemon.join("dirs/work.dir"), workspace.to_str().unwrap()).unwrap();
    (fixture, server)
}

fn deletion(client: &mut Client, confirmed: bool) -> serde_json::Value {
    client.request(
        "tools/call",
        json!({"name":"delete_provider","arguments":{"name":"message","confirmed":confirmed}}),
    )
}

fn delete_when_idle(client: &mut Client) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = deletion(client, true);
        if !result.to_string().contains("rule worker is active") {
            return result;
        }
        assert!(Instant::now() < deadline, "{result}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn mcp_provider_deletion_requires_approval_and_purges_only_its_owned_resources_and_history() {
    let (fixture, server) = fixture();
    let mut client = Client::with_command(fixture.command(&["mcp"]));
    let refused = deletion(&mut client, false);
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(refused.to_string().contains("confirmation_required"));
    assert!(fixture.home.join("templates/providers/message").exists());
    assert!(fixture.home.join("workspaces/review").exists());
    assert!(server.data.lock().unwrap().deleted.is_empty());
    let result = delete_when_idle(&mut client);
    assert_ne!(result["isError"], true, "{result}");
    for path in [
        "templates/providers/message",
        "runtime/cli-test/providers/message",
        "provider-credentials/cli-test/dev-message.token",
        "workspaces/review",
    ] {
        assert!(!fixture.home.join(path).exists(), "{path}");
    }
    assert!(fixture.home.join("templates/providers/ticket").exists());
    assert_eq!(server.data.lock().unwrap().deleted, ["ses_provider"]);
    assert!(
        server
            .data
            .lock()
            .unwrap()
            .sessions
            .contains_key("ses_unrelated")
    );
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    for query in [
        "SELECT count(*) FROM provider_launches WHERE namespace='cli-test' AND name='message'",
        "SELECT count(*) FROM events WHERE namespace='cli-test' AND provider='dev-message'",
        "SELECT count(*) FROM event_providers WHERE namespace='cli-test' AND name='dev-message'",
        "SELECT count(*) FROM provider_streams WHERE namespace='cli-test' AND provider='dev-message'",
        "SELECT count(*) FROM provider_ingestion WHERE namespace='cli-test' AND name='dev-message'",
        "SELECT count(*) FROM provider_notifications WHERE namespace='cli-test' AND provider='dev-message'",
        "SELECT count(*) FROM rule_acceptances WHERE attempt_id=1",
        "SELECT count(*) FROM rule_evaluations WHERE attempt_id=1",
        "SELECT count(*) FROM event_attempts WHERE event_sequence=1",
        "SELECT count(*) FROM runtime_records WHERE namespace='cli-test' AND name='review'",
        "SELECT count(*) FROM provider_deletions WHERE namespace='cli-test' AND name='message'",
    ] {
        assert_eq!(
            connection
                .query_row(query, [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0,
            "{query}"
        );
    }
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(fixture.home.join("collector-removed").exists());
    assert!(fixture.home.join("volume-removed").exists());
    let providers = client.tool("list_providers", json!({}));
    assert!(
        !providers["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "message")
    );
}

#[test]
fn provider_cleanup_keeps_acceptance_ownership_through_scoped_start_and_legacy_startup_records() {
    for legacy in [false, true] {
        let (fixture, _server) = fixture();
        let original =
            fixture.runtime_record("review", "startup").unwrap()["operation"]["id"].clone();
        if legacy {
            let mut record = fixture.runtime_record("review", "startup").unwrap();
            record
                .as_object_mut()
                .unwrap()
                .remove("origin_operation_id");
            fixture.set_runtime_record("review", "startup", &record);
        }
        let mut command = fixture.command(&["mcp-instance"]);
        command.current_dir(fixture.home.join("workspaces/review"));
        let mut scoped = Client::with_command(command);
        assert_eq!(scoped.tool("start_self", json!({}))["state"], "succeeded");
        let started = fixture.runtime_record("review", "startup").unwrap();
        assert_ne!(started["operation"]["id"], original);
        assert_eq!(started["origin_operation_id"], original);
        let mut management = Client::with_command(fixture.command(&["mcp"]));
        let result = delete_when_idle(&mut management);
        assert_ne!(result["isError"], true, "{result}");
        assert!(!fixture.home.join("workspaces/review").exists());
    }
}

#[test]
fn mcp_provider_deletion_preserves_retry_evidence_after_external_failure() {
    let (fixture, server) = fixture();
    fs::write(fixture.home.join("reject-volume-removal"), "").unwrap();
    let mut client = Client::with_command(fixture.command(&["mcp"]));
    let result = delete_when_idle(&mut client);
    assert_eq!(result["isError"], true, "{result}");
    assert!(fixture.home.join("templates/providers/message").exists());
    assert!(fixture.home.join("workspaces/review").exists());
    let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
    assert!(!connection.query_row("SELECT enabled FROM provider_ingestion WHERE namespace='cli-test' AND name='dev-message'", [], |row| row.get::<_, bool>(0)).unwrap());
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM events WHERE namespace='cli-test' AND provider='dev-message'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let restart = client.request("tools/call", json!({"name":"provider_action","arguments":{"name":"message","action":"start","confirmed":true}}));
    assert_eq!(restart["isError"], true, "{restart}");
    assert!(
        restart.to_string().contains("deletion is incomplete"),
        "{restart}"
    );
    fs::remove_file(fixture.home.join("reject-volume-removal")).unwrap();
    server.data.lock().unwrap().delete_failure = true;
    let result = delete_when_idle(&mut client);
    assert_eq!(result["isError"], true, "{result}");
    assert!(result.to_string().contains("500"), "{result}");
    assert!(fixture.home.join("workspaces/review").exists());
    server.data.lock().unwrap().delete_failure = false;
    let result = delete_when_idle(&mut client);
    assert_ne!(result["isError"], true, "{result}");
    assert!(!fixture.home.join("templates/providers/message").exists());
}

#[test]
fn mcp_provider_deletion_removes_saved_sessions_after_the_created_instance_is_absent() {
    let (fixture, server) = fixture();
    let output = fixture.run(&["delete-instance", "review"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.home.join("workspaces/review").exists());
    assert!(server.data.lock().unwrap().deleted.is_empty());
    let mut client = Client::with_command(fixture.command(&["mcp"]));
    let result = delete_when_idle(&mut client);
    assert_ne!(result["isError"], true, "{result}");
    assert_eq!(server.data.lock().unwrap().deleted, ["ses_provider"]);
}

#[test]
fn mcp_provider_deletion_refuses_shared_packages_active_dispatches_and_reused_instances() {
    for reason in ["namespace", "active", "reused", "volume", "symlink"] {
        let (fixture, server) = fixture();
        let connection = rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap();
        match reason {
            "namespace" => {
                connection.execute("INSERT INTO provider_launches SELECT 'other-test',name,directory,manifest,compose,error FROM provider_launches WHERE namespace='cli-test' AND name='message'", []).unwrap();
            }
            "active" => {
                connection.execute("UPDATE rule_acceptances SET payload=json_set(payload,'$.status','provisioning') WHERE id=1", []).unwrap();
            }
            "reused" => {
                assert!(fixture.run(&["delete-instance", "review"]).status.success());
                assert!(
                    fixture
                        .run(&["new-instance", "review", "-t", "blank"])
                        .status
                        .success()
                );
            }
            "volume" => {
                fs::write(fixture.home.join("foreign-volume"), "").unwrap();
            }
            "symlink" => {
                let package = fixture.home.join("templates/providers/message");
                fs::remove_dir_all(&package).unwrap();
                std::os::unix::fs::symlink(&fixture.source, package).unwrap();
            }
            _ => unreachable!(),
        }
        let mut client = Client::with_command(fixture.command(&["mcp"]));
        let result = delete_when_idle(&mut client);
        assert_eq!(result["isError"], true, "{reason}: {result}");
        let expected = match reason {
            "namespace" => "another namespace",
            "active" => "active rule actions",
            "reused" => "different startup identity",
            "volume" => "unverifiable ownership",
            "symlink" => "symlink",
            _ => unreachable!(),
        };
        assert!(result.to_string().contains(expected), "{reason}: {result}");
        assert!(fixture.home.join("workspaces/review").exists(), "{reason}");
        assert!(
            fixture.home.join("templates/providers/message").exists(),
            "{reason}"
        );
        assert!(server.data.lock().unwrap().deleted.is_empty(), "{reason}");
        assert_eq!(connection.query_row("SELECT count(*) FROM events WHERE namespace='cli-test' AND provider='dev-message'", [], |row| row.get::<_, i64>(0)).unwrap(), 1, "{reason}");
    }
}

const DOCKER: &str = r#"#!/bin/sh
case "$1" in
 ps)
  case "$*" in *cli-test-provider-message*) test -f "$TANDEM_HOME/collector-removed" || printf 'fixture-provider\n' ;; esac ;;
 inspect)
  running=true; test ! -f "$TANDEM_HOME/provider-stopped" || running=false
  printf '[{"Id":"fixture-provider","Config":{"Labels":{"io.tandem.provider-namespace":"cli-test","io.tandem.provider-name":"message","com.docker.compose.project":"cli-test-provider-message","com.docker.compose.service":"collector"}},"State":{"Running":%s,"Paused":false}}]\n' "$running" ;;
 volume)
  case "$2" in
   ls) test -f "$TANDEM_HOME/volume-removed" || printf 'cli-test-provider-message_state\n' ;;
   inspect)
    namespace=cli-test; test ! -f "$TANDEM_HOME/foreign-volume" || namespace=other-test
    printf '[{"Name":"cli-test-provider-message_state","Labels":{"io.tandem.provider-namespace":"%s","io.tandem.provider-name":"message"}}]\n' "$namespace" ;;
   rm) test ! -f "$TANDEM_HOME/reject-volume-removal" || exit 1; touch "$TANDEM_HOME/volume-removed" ;;
  esac ;;
 stop) touch "$TANDEM_HOME/provider-stopped" ;;
 rm) touch "$TANDEM_HOME/collector-removed" ;;
esac
"#;
