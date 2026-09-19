use super::*;

#[test]
fn deletion_runs_the_saved_close_command_and_reports_failure_without_stopping_cleanup() {
    for flag in ["--close-command", "-cc"] {
        let fixture = Fixture::new();
        let workspace = fixture.home.join("workspaces/review");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("data"), "local work").unwrap();
        fs::write(fixture.home.join("started"), "").unwrap();
        rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap().execute(
        "INSERT INTO app_settings(key, value) VALUES ('instances.close_command', ?1)",
        ["test -f data || exit 12; printf '%s\\n' \"$TANDEM_INSTANCE\" \"$TANDEM_WORKSPACE\" \"$PWD\" > \"$TANDEM_HOME/closed\"; printf 'private output'; printf 'private error' >&2; exit 23"],
    ).unwrap();

        let output = fixture.run(&["delete-instance", "review", flag]);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!workspace.exists());
        assert_eq!(
            fs::read_to_string(fixture.home.join("closed")).unwrap(),
            format!("review\n{}\n{}\n", workspace.display(), workspace.display())
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Close command for review failed"),
            "{stderr}"
        );
        assert!(stderr.contains("23"), "{stderr}");
        assert!(!stderr.contains("private error"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private output"));
    }
}
