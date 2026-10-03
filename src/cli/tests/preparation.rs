use super::*;

fn prepare(fixture: &Fixture) {
    let output = fixture.run(&[
        "new-instance",
        "review",
        "-t",
        "website",
        "--start-instance=false",
        "-o",
        "Explore the code",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let startup = fixture.runtime_record("review", "startup").unwrap();
    assert_eq!(startup["start_instance"], false);
    assert_eq!(startup["operation"]["state"], "succeeded");
    assert_eq!(startup["operation"]["instance"]["workspace_only"], false);
    assert!(fixture.home.join("configured").is_file());
    assert!(
        fixture
            .home
            .join("workspaces/review/app/file.txt")
            .is_file()
    );
    assert!(fixture.home.join("workspaces/review/AGENTS.md").is_file());
    assert!(!fixture.home.join("started").exists());
    let calls = fs::read_to_string(fixture.home.join("docker-calls")).unwrap();
    assert!(!calls.contains("up --detach"), "{calls}");
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened-parameters")).unwrap(),
        "Explore the code"
    );
    assert!(
        fs::read_to_string(fixture.home.join("opened-instructions"))
            .unwrap()
            .contains("Services won't start automatically")
    );
}

#[test]
fn prepared_service_instances_survive_inventory_and_can_start_later() {
    let fixture = Fixture::new();
    prepare(&fixture);
    let output = fixture.run(&["list-instances", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inventory: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let instance = &inventory["instances"][0];
    assert_eq!(instance["name"], "review");
    assert_eq!(instance["workspace_only"], false);
    assert_eq!(instance["summary"]["status"], "not_started");
    let output = fixture.run(&["new-instance", "review", "-t", "website", "-o"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.home.join("started").exists());
    assert!(
        fs::read_to_string(fixture.home.join("opened-instructions"))
            .unwrap()
            .contains("Services won't start automatically")
    );
    let output = fixture.run(&["start-instance", "review"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fixture.home.join("started").exists());
    let output = fixture.run(&[
        "new-instance",
        "review",
        "-t",
        "website",
        "--start-instance=false",
        "-o",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(fixture.home.join("opened-instructions"))
            .unwrap()
            .contains("Services are started")
    );
}

#[test]
fn prepared_service_instances_can_be_deleted_without_starting_containers() {
    let fixture = Fixture::new();
    prepare(&fixture);
    fs::remove_dir_all(fixture.home.join("templates/website")).unwrap();
    let output = fixture.run(&["delete-instance", "review"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.home.join("workspaces/review").exists());
    assert!(fixture.runtime_record("review", "journal").is_none());
    assert!(!fixture.home.join("started").exists());
}
