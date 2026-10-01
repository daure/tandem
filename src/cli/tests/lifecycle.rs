use super::*;

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn running_fixture() -> Fixture {
    let fixture = Fixture::new();
    success(fixture.run(&[
        "new-instance",
        "review",
        "-t",
        "website",
        "-d",
        "Review workspace",
    ]));
    let inspection: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.home.join("inspect.json")).unwrap())
            .unwrap();
    let mut stopped = inspection.clone();
    stopped[0]["State"]["Status"] = json!("exited");
    stopped[0]["State"]["Running"] = json!(false);
    stopped[0]["State"]["ExitCode"] = json!(0);
    stopped[0]["State"]["FinishedAt"] = json!("2026-01-01T00:01:00Z");
    fs::write(fixture.home.join("stopped.json"), stopped.to_string()).unwrap();
    let mut running = inspection;
    running[0]["State"]["StartedAt"] = json!("2026-01-01T00:02:00Z");
    fs::write(fixture.home.join("running.json"), running.to_string()).unwrap();
    fs::write(fixture.bin.join("docker-base"), DOCKER).unwrap();
    fs::set_permissions(
        fixture.bin.join("docker-base"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::write(fixture.bin.join("docker"), LIFECYCLE_DOCKER).unwrap();
    fixture
}

#[test]
fn instance_creation_builds_the_selected_template_images() {
    let fixture = Fixture::new();
    success(fixture.run(&["new-instance", "review", "-t", "website"]));
    let calls = fs::read_to_string(fixture.home.join("docker-calls")).unwrap();
    let startup = calls
        .lines()
        .find(|line| line.contains("--project-name cli-test-review") && line.contains(" up "))
        .unwrap();
    assert!(
        startup
            .split_whitespace()
            .any(|argument| argument == "--build"),
        "{startup}"
    );
}

#[test]
fn stopping_starting_and_restarting_preserve_workspace_changes_and_report_completion() {
    let fixture = running_fixture();
    let checkout = fixture.home.join("workspaces/review/app");
    fs::write(checkout.join("file.txt"), "Local edits").unwrap();
    let rendered = fixture.home.join("runtime/cli-test/review/compose.json");

    let stopped = success(fixture.run(&["stop-instance", "review"]));
    assert_eq!(stopped, "Instance review stopped\n");
    assert!(rendered.exists());
    let inspection: serde_json::Value = serde_json::from_str(&success(fixture.run(&[
        "inspect-instance",
        "review",
        "--json",
    ])))
    .unwrap();
    assert_eq!(inspection["instance"]["summary"]["status"], "stopped");

    fs::remove_file(fixture.home.join("configured")).unwrap();
    let started = success(fixture.run(&["start-instance", "review"]));
    assert!(
        started.starts_with("Instance review ready\nWorkspace:"),
        "{started}"
    );
    assert!(fixture.home.join("configured").exists());
    assert_eq!(
        fs::read_to_string(checkout.join("file.txt")).unwrap(),
        "Local edits"
    );
    assert_eq!(git(&checkout, &["branch", "--show-current"]), "review");

    let before_restart = fs::read(&rendered).unwrap();
    fs::write(fixture.home.join("templates/website/tandem.json"), "{").unwrap();
    let restarted = success(fixture.run(&["restart-instance", "review"]));
    assert_eq!(restarted, "Instance review restarted\n");
    assert_eq!(fs::read(&rendered).unwrap(), before_restart);
    assert_eq!(
        fs::read_to_string(checkout.join("file.txt")).unwrap(),
        "Local edits"
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("container-actions")).unwrap(),
        "stop fixture-container\nrestart fixture-container\n"
    );
}

#[test]
fn restart_targets_long_running_containers_and_skips_setup_jobs() {
    let fixture = running_fixture();
    let mut containers: Vec<serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(fixture.home.join("inspect.json")).unwrap())
            .unwrap();
    let mut setup = containers[0].clone();
    setup["Id"] = json!("setup-container");
    setup["Config"]["Labels"]["com.docker.compose.service"] = json!("setup");
    setup["Config"]["Labels"]["io.tandem.role"] = json!("oneshot");
    setup["State"]["Status"] = json!("exited");
    setup["State"]["Running"] = json!(false);
    setup["State"]["ExitCode"] = json!(0);
    containers.push(setup);
    for path in ["inspect.json", "running.json"] {
        fs::write(
            fixture.home.join(path),
            serde_json::to_string(&containers).unwrap(),
        )
        .unwrap();
    }
    fs::write(fixture.home.join("include-setup"), "").unwrap();

    success(fixture.run(&["restart-instance", "review"]));
    assert_eq!(
        fs::read_to_string(fixture.home.join("container-actions")).unwrap(),
        "restart fixture-container\n"
    );
}

#[test]
fn lifecycle_command_failures_exit_nonzero_and_preserve_workspace_data() {
    let fixture = running_fixture();
    let data = fixture.home.join("workspaces/review/data.txt");
    fs::write(&data, "Keep data").unwrap();
    for (command, flag, expected) in [
        ("start-instance", "FAIL_START", "fixture startup failed"),
        ("stop-instance", "FAIL_STOP", "fixture stop failed"),
        ("restart-instance", "FAIL_RESTART", "fixture restart failed"),
    ] {
        let output = wait_output(
            fixture
                .command(&[command, "review"])
                .env(flag, "1")
                .spawn()
                .unwrap(),
        );
        assert!(!output.status.success(), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "{error}");
        assert_eq!(fs::read_to_string(&data).unwrap(), "Keep data");
    }
    let inspection: serde_json::Value = serde_json::from_str(&success(fixture.run(&[
        "inspect-instance",
        "review",
        "--json",
    ])))
    .unwrap();
    assert_eq!(inspection["startup"]["state"], "failed");
    assert!(
        inspection["startup"]["error"]
            .as_str()
            .unwrap()
            .contains("fixture startup failed")
    );
    assert!(
        inspection["activities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|activity| {
                activity["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("fixture restart failed"))
            })
    );
}

#[test]
fn lifecycle_commands_refuse_unmanaged_project_membership_before_mutation() {
    let fixture = running_fixture();
    for command in ["start-instance", "stop-instance", "restart-instance"] {
        let output = wait_output(
            fixture
                .command(&[command, "review"])
                .env("UNMANAGED", "1")
                .spawn()
                .unwrap(),
        );
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("project contains unmanaged containers"),
            "{error}"
        );
    }
    assert!(!fixture.home.join("container-actions").exists());
}

#[test]
fn named_instance_commands_reject_missing_instances_and_reserved_names() {
    let fixture = Fixture::new();
    for name in ["missing", "gateway", "../escape"] {
        for command in [
            "inspect-instance",
            "start-instance",
            "stop-instance",
            "restart-instance",
        ] {
            let output = fixture.run(&[command, name]);
            assert!(!output.status.success(), "{command} {name}");
            assert!(output.stdout.is_empty(), "{command} {name}");
        }
    }
    assert!(!fixture.home.join("workspaces/missing").exists());
    assert!(!fixture.home.join("configured").exists());
}

#[test]
fn instance_start_requires_matching_ownership_and_unpaused_containers() {
    let fixture = running_fixture();
    fs::remove_file(fixture.home.join("configured")).unwrap();
    let original: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.home.join("inspect.json")).unwrap())
            .unwrap();
    let mut paused = original.clone();
    paused[0]["State"]["Status"] = json!("paused");
    paused[0]["State"]["Paused"] = json!(true);
    let mut different_workspace = original.clone();
    different_workspace[0]["Config"]["Labels"]["io.tandem.workspace"] = json!("/foreign/workspace");
    for (containers, expected) in [
        (paused, "unpause the instance containers before start"),
        (
            different_workspace,
            "instance belongs to a different template directory, workspace, or execution kind",
        ),
    ] {
        fs::write(fixture.home.join("inspect.json"), containers.to_string()).unwrap();
        let output = fixture.run(&["start-instance", "review"]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "{error}");
    }
    fs::write(fixture.home.join("inspect.json"), original.to_string()).unwrap();
    fs::remove_file(fixture.home.join("templates/website/compose.yaml")).unwrap();
    let output = fixture.run(&["start-instance", "review"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("execution kind"));
    assert!(!fixture.home.join("configured").exists());
    assert!(!fixture.home.join("container-actions").exists());
}

#[test]
fn workspace_start_and_stop_preserve_files_without_docker_and_restart_reports_no_services() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.home.join("templates/website/compose.yaml")).unwrap();
    fs::write(fixture.home.join("templates/website/tandem.json"), "{}").unwrap();
    fs::write(fixture.bin.join("docker"), "#!/bin/sh\nexit 99\n").unwrap();
    success(fixture.run(&["new-instance", "scratch", "-t", "website"]));
    let data = fixture.home.join("workspaces/scratch/notes.txt");
    fs::write(&data, "Keep notes").unwrap();
    success(fixture.run(&["stop-instance", "scratch"]));
    success(fixture.run(&["start-instance", "scratch"]));
    let restarted = fixture.run(&["restart-instance", "scratch"]);
    assert!(!restarted.status.success());
    assert!(
        String::from_utf8_lossy(&restarted.stderr)
            .contains("instance has no long-running services")
    );
    assert_eq!(fs::read_to_string(data).unwrap(), "Keep notes");
}

const LIFECYCLE_DOCKER: &str = r#"#!/bin/sh
case "$1" in
  ps)
    case "$*" in
      *cli-test-gateway*) exit 0;;
    esac
    if [ -f "$TANDEM_HOME/started" ]; then printf 'fixture-container\n'; fi
    if [ -f "$TANDEM_HOME/include-setup" ]; then printf 'setup-container\n'; fi
    case "$*" in
      *com.docker.compose.project=cli-test-review*)
        if [ "$UNMANAGED" = 1 ]; then printf 'foreign-container\n'; fi;;
    esac
    ;;
  stop)
    if [ "$FAIL_STOP" = 1 ]; then printf 'fixture stop failed\n' >&2; exit 23; fi
    printf '%s\n' "$*" >> "$TANDEM_HOME/container-actions"
    cp "$TANDEM_HOME/stopped.json" "$TANDEM_HOME/inspect-next.json"
    mv "$TANDEM_HOME/inspect-next.json" "$TANDEM_HOME/inspect.json"
    ;;
  restart)
    if [ "$FAIL_RESTART" = 1 ]; then printf 'fixture restart failed\n' >&2; exit 23; fi
    printf '%s\n' "$*" >> "$TANDEM_HOME/container-actions"
    cp "$TANDEM_HOME/running.json" "$TANDEM_HOME/inspect-next.json"
    mv "$TANDEM_HOME/inspect-next.json" "$TANDEM_HOME/inspect.json"
    ;;
  compose)
    case "$*" in
      *'up --detach'*)
        if [ "$TANDEM_INSTANCE" != gateway ]; then
          cp "$TANDEM_HOME/running.json" "$TANDEM_HOME/inspect-next.json"
          mv "$TANDEM_HOME/inspect-next.json" "$TANDEM_HOME/inspect.json"
        fi;;
    esac
    exec "$(dirname "$0")/docker-base" "$@"
    ;;
  *) exec "$(dirname "$0")/docker-base" "$@";;
esac
"#;
