use std::{
    fs,
    sync::{Arc, Mutex},
};

use super::*;
use crate::environments::{config::Config, lifecycle};

#[test]
fn workspace_removal_waits_for_close_and_preserves_literal_environment_values() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home ' ; $(touch injected)");
    let config = Config::at(home, "test".into(), 9876).unwrap();
    let workspace = config.workspaces.join("Review");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("data"), "local work").unwrap();
    let close = CloseCommand::new(Ok("test -f data || exit 12; printf '%s\\n' \"$TANDEM_INSTANCE\" \"$TANDEM_WORKSPACE\" \"$PWD\" > ../closed".into()), Arc::new(|warning| panic!("{warning}")));

    lifecycle::remove_workspace(&config, "Review", Arc::new(|_| {}), &close).unwrap();

    assert!(!workspace.exists());
    assert_eq!(
        fs::read_to_string(config.workspaces.join("closed")).unwrap(),
        format!("Review\n{}\n{}\n", workspace.display(), workspace.display())
    );
    assert!(!config.workspaces.join("injected").exists());
}

#[test]
fn close_failures_warn_and_workspace_deletion_continues() {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().to_owned(), "test".into(), 9876).unwrap();
    for command in [Ok("exit 23".into()), Err("settings unavailable".into())] {
        let workspace = config.workspaces.join("review");
        fs::create_dir(&workspace).unwrap();
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let received = warnings.clone();
        let close = CloseCommand::new(
            command,
            Arc::new(move |warning| received.lock().unwrap().push(warning)),
        );
        lifecycle::remove_workspace(&config, "review", Arc::new(|_| {}), &close).unwrap();
        assert!(!workspace.exists());
        let warnings = warnings.lock().unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Close command for review failed"));
        assert!(warnings[0].contains("continuing deletion"));
    }
}

#[test]
fn missing_or_unsafe_workspaces_do_not_run_close_commands() {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().to_owned(), "test".into(), 9876).unwrap();
    let close = CloseCommand::new(
        Ok("touch ../unexpected".into()),
        Arc::new(|warning| panic!("{warning}")),
    );
    lifecycle::remove_workspace(&config, "missing", Arc::new(|_| {}), &close).unwrap();
    #[cfg(unix)]
    {
        let outside = directory.path().join("outside/target");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, config.workspaces.join("linked")).unwrap();
        assert!(lifecycle::remove_workspace(&config, "linked", Arc::new(|_| {}), &close).is_err());
        assert!(!outside.parent().unwrap().join("unexpected").exists());
    }
    assert!(!config.workspaces.join("unexpected").exists());
}

#[test]
fn close_commands_have_a_bounded_wait() {
    let directory = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let error = execute(
        "sleep 5",
        "review",
        directory.path(),
        Duration::from_millis(40),
    )
    .unwrap_err();
    assert_eq!(error, "timed out");
    assert!(started.elapsed() < Duration::from_secs(1));
}
