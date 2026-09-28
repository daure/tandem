use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

use super::client_command;

const PROMPT: &str = "Explain 'this'; $(touch injected)\nsecond line";
const WORKSPACE: &str = "workspace ' ; $(touch injected)";

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn launch(direnv: Option<&str>) -> (tempfile::TempDir, std::process::Output) {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    let workspace = root.path().join(WORKSPACE);
    fs::create_dir(&bin).unwrap();
    fs::create_dir(&workspace).unwrap();
    if let Some(script) = direnv {
        executable(&bin.join("direnv"), script);
    }
    executable(
        &bin.join("opencode"),
        "#!/bin/sh\nprintf '%s\\0' \"${OPENCODE_CONFIG_DIR-unset}\" \"$PWD\" \"$TANDEM_INITIAL_PROMPT\" \"$@\"\nexit 23\n",
    );
    let command = client_command(&[
        "/usr/bin/env".into(),
        format!("TANDEM_INITIAL_PROMPT={PROMPT}"),
        "opencode".into(),
        "attach".into(),
        "http://127.0.0.1:4199".into(),
        "--dir".into(),
        workspace.display().to_string(),
        "--session".into(),
        "ses_saved".into(),
        String::new(),
    ]);
    let output = Command::new(&command[0])
        .args(&command[1..])
        .env_clear()
        .env("HOME", root.path())
        .env("PATH", &bin)
        .current_dir(&workspace)
        .output()
        .unwrap();
    assert!(!workspace.join("injected").exists());
    (root, output)
}

#[test]
fn client_launches_preserve_arguments_with_optional_direnv() {
    let direnv = "#!/bin/sh\n[ \"$1\" = exec ] && [ \"$2\" = . ] || exit 41\nshift 2\nexport OPENCODE_CONFIG_DIR=docked-station\nexec \"$@\"\n";
    for (script, station) in [(None, "unset"), (Some(direnv), "docked-station")] {
        let (root, output) = launch(script);
        assert_eq!(output.status.code(), Some(23), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let workspace = root.path().join(WORKSPACE).display().to_string();
        let expected = [
            station,
            workspace.as_str(),
            PROMPT,
            "attach",
            "http://127.0.0.1:4199",
            "--dir",
            workspace.as_str(),
            "--session",
            "ses_saved",
            "",
        ];
        assert_eq!(
            output.stdout,
            format!("{}\0", expected.join("\0")).as_bytes()
        );
    }
}

#[test]
fn direnv_failure_stops_the_client_launch() {
    let (_root, output) = launch(Some(
        "#!/bin/sh\nprintf 'environment blocked\\n' >&2\nexit 42\n",
    ));
    assert_eq!(output.status.code(), Some(42));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"environment blocked\n");
}
