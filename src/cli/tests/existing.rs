use super::*;

fn existing_fixture(state: &str) -> Fixture {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.home.join("workspaces/review")).unwrap();
    fs::write(fixture.home.join("workspaces/review/keep"), "local work").unwrap();
    fs::write(fixture.home.join("started"), "").unwrap();
    let path = fixture.home.join("inspect.json");
    let mut containers: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    containers[0]["State"]["Status"] = json!(state);
    containers[0]["State"]["Running"] = json!(state == "running");
    containers[0]["State"]["ExitCode"] = json!(0);
    fs::write(path, containers.to_string()).unwrap();
    // Opening an existing workspace is independent of template files and source access.
    fs::remove_dir_all(fixture.home.join("templates/website")).unwrap();
    fs::remove_dir_all(&fixture.source).unwrap();
    fixture.save_command(
        "printf '%s\\n%s\\n' \"$TANDEM_INSTANCE\" \"$PWD\" >> \"$TANDEM_HOME/opened\"",
    );
    fixture
}

fn assert_inspection_only(fixture: &Fixture) {
    let calls = fs::read_to_string(fixture.home.join("docker-calls")).unwrap();
    assert!(!calls.is_empty());
    assert!(
        calls
            .lines()
            .all(|line| line.starts_with("ps ") || line.starts_with("inspect ")),
        "{calls}"
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("workspaces/review/keep")).unwrap(),
        "local work"
    );
}

#[test]
fn existing_running_and_stopped_instances_are_unchanged_with_optional_opening() {
    for state in ["running", "exited", "paused"] {
        for flag in [None, Some("-oc"), Some("--open-command")] {
            let fixture = existing_fixture(state);
            let mut args = vec!["new-instance", "review", "-t", "website"];
            args.extend(flag);
            let output = fixture.run(&args);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.contains("already exists; left unchanged"),
                "{stdout}"
            );
            assert!(!stdout.contains("Instance review ready"), "{stdout}");
            assert_inspection_only(&fixture);
            if flag.is_some() {
                let opened = fixture.home.join("opened");
                let expected = format!(
                    "review\n{}\n",
                    fixture.home.join("workspaces/review").display()
                );
                let deadline = Instant::now() + Duration::from_secs(5);
                while fs::read_to_string(&opened).ok().as_deref() != Some(&expected) {
                    assert!(Instant::now() < deadline, "opener did not run");
                    thread::sleep(Duration::from_millis(20));
                }
            } else {
                assert!(!fixture.home.join("opened").exists());
            }
        }
    }
}

#[test]
fn creation_keeps_the_instance_lock_between_admission_and_startup() {
    let fixture = Fixture::new();
    let first = fixture
        .command(&["new-instance", "review", "-t", "website"])
        .env("BLOCK_CONFIG", "1")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fixture.home.join("configured").exists() {
        assert!(Instant::now() < deadline, "startup did not reach Compose");
        thread::sleep(Duration::from_millis(20));
    }
    let second = fixture.run(&["new-instance", "review", "-t", "website", "-oc"]);
    fs::write(fixture.home.join("release-config"), "").unwrap();
    let first = wait_output(first);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("busy"));
    assert!(!fixture.home.join("opened").exists());
}

#[test]
fn an_existing_instance_requires_matching_ownership_before_opening() {
    for mismatch in ["template", "workspace", "unmanaged"] {
        let fixture = existing_fixture("running");
        let path = fixture.home.join("inspect.json");
        let mut containers: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let labels = &mut containers[0]["Config"]["Labels"];
        match mismatch {
            "template" => labels["io.tandem.template"] = json!("other"),
            "workspace" => labels["io.tandem.workspace"] = json!("/unrelated/workspace"),
            _ => labels["io.tandem.namespace"] = json!("other"),
        }
        fs::write(path, containers.to_string()).unwrap();
        let output = fixture.run(&["new-instance", "review", "-t", "website", "-oc"]);
        assert!(!output.status.success(), "{mismatch}");
        assert!(!fixture.home.join("opened").exists());
        assert_inspection_only(&fixture);
    }
}

#[test]
fn existing_instances_with_missing_workspaces_are_only_rejected_when_opening() {
    let fixture = existing_fixture("exited");
    fs::remove_dir_all(fixture.home.join("workspaces/review")).unwrap();
    assert!(
        fixture
            .run(&["new-instance", "review", "-t", "website"])
            .status
            .success()
    );
    let output = fixture.run(&["new-instance", "review", "-t", "website", "-oc"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("workspace review"));
    assert!(!fixture.home.join("opened").exists());
    assert!(!fixture.home.join("configured").exists());
}
