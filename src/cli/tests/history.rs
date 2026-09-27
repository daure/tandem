use super::*;

#[path = "../../environments/opencode/tests/history_server.rs"]
mod history_server;
use history_server::Server;

fn fixture(discoverable: bool) -> (Fixture, Server) {
    let fixture = Fixture::new();
    fs::create_dir(fixture.home.join("templates/blank")).unwrap();
    fs::write(fixture.home.join("templates/blank/tandem.json"), "{}").unwrap();
    rusqlite::Connection::open(fixture.home.join("settings.sqlite3"))
        .unwrap()
        .execute(
            "DELETE FROM app_settings WHERE key = 'opencode.clear_history_on_creation'",
            [],
        )
        .unwrap();
    let server = Server::start();
    let workspace = fixture.home.join("workspaces/review");
    server.session("ses_old", workspace.to_str().unwrap(), None);
    if discoverable {
        let daemon = fixture.home.join("daemons/test");
        fs::create_dir_all(daemon.join("dirs")).unwrap();
        fs::write(daemon.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
        fs::write(daemon.join("dirs/work.dir"), workspace.to_str().unwrap()).unwrap();
    }
    (fixture, server)
}

fn create(fixture: &Fixture) -> Output {
    fixture.run(&["new-instance", "review", "-t", "blank"])
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn default_on_cleanup_runs_for_creation_and_recreation_but_preserves_existing_instances() {
    let (fixture, server) = fixture(true);
    success(create(&fixture));
    assert_eq!(server.data.lock().unwrap().deleted, ["ses_old"]);
    server.session(
        "ses_new",
        fixture.home.join("workspaces/review").to_str().unwrap(),
        None,
    );
    success(create(&fixture));
    assert!(server.data.lock().unwrap().sessions.contains_key("ses_new"));
    success(fixture.run(&["delete-instance", "review"]));
    success(create(&fixture));
    assert_eq!(server.data.lock().unwrap().deleted, ["ses_old", "ses_new"]);
}

#[test]
fn either_disabled_cleanup_or_disabled_integration_preserves_saved_history() {
    for key in [
        "opencode.clear_history_on_creation",
        "integrations.opencode",
    ] {
        let (fixture, server) = fixture(true);
        rusqlite::Connection::open(fixture.home.join("settings.sqlite3"))
            .unwrap()
            .execute(
                "INSERT INTO app_settings(key, value) VALUES (?1, 'false')",
                [key],
            )
            .unwrap();
        success(create(&fixture));
        assert!(server.data.lock().unwrap().sessions.contains_key("ses_old"));
        assert!(server.data.lock().unwrap().deleted.is_empty());
    }
}

#[test]
fn active_history_blocks_readiness_and_cleanup_is_retried_after_it_becomes_idle() {
    let (fixture, server) = fixture(true);
    server.data.lock().unwrap().busy = true;
    let output = create(&fixture);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("conversations are active"));
    assert!(!fixture.home.join("workspaces/review/AGENTS.md").exists());
    assert!(server.data.lock().unwrap().deleted.is_empty());
    server.data.lock().unwrap().busy = false;
    success(create(&fixture));
    assert_eq!(server.data.lock().unwrap().deleted, ["ses_old"]);
}

#[test]
fn undiscovered_history_uses_a_temporary_server_and_stops_it_after_cleanup() {
    for fails in [false, true] {
        let (fixture, server) = fixture(false);
        server.data.lock().unwrap().delete_failure = fails;
        fs::write(fixture.bin.join("opencode"), format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > \"$TANDEM_HOME/cleanup-pid\"\nprintf '%s\\n' \"$*\" > \"$TANDEM_HOME/cleanup-args\"\nprintf 'opencode server listening on {}\\n'\nexec sleep 60\n", server.url,
        )).unwrap();
        let output = create(&fixture);
        if fails {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("500"));
        } else {
            success(output);
            assert_eq!(server.data.lock().unwrap().deleted, ["ses_old"]);
        }
        assert_eq!(
            fs::read_to_string(fixture.home.join("cleanup-args")).unwrap(),
            "serve --hostname 127.0.0.1 --port 0\n"
        );
        let pid = fs::read_to_string(fixture.home.join("cleanup-pid")).unwrap();
        assert!(!Path::new("/proc").join(pid.trim()).exists());
    }
}
