use super::*;

fn damaged_workspace() -> Fixture {
    let fixture = Fixture::new();
    let template = fixture.home.join("templates/website");
    fs::remove_file(template.join("compose.yaml")).unwrap();
    fs::remove_file(template.join("tandem.json")).unwrap();
    fs::write(
        template.join("tandem-agents.md"),
        "## Notes\n\nKeep notes locally.\n",
    )
    .unwrap();
    let output = fixture.run(&["new-instance", "review", "-t", "website"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = fixture.home.join("runtime/cli-test/review.json");
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let expected = record["expected"].as_object_mut().unwrap();
    expected.remove("workspace_only");
    expected.remove("repositories");
    expected["runtime"]
        .as_object_mut()
        .unwrap()
        .remove("workspace_ready");
    record.as_object_mut().unwrap().remove("repositories");
    record["activity"]["action"] = "delete_instance".into();
    record["activity"]["error"] = "instance not found".into();
    record["activity"]["finished"] = true.into();
    fs::write(path, record.to_string()).unwrap();
    fs::write(
        fixture.home.join("workspaces/review/notes.md"),
        "workspace notes",
    )
    .unwrap();
    fixture
}

#[test]
fn deletion_recovers_owned_workspaces_when_the_runtime_kind_is_missing() {
    let fixture = damaged_workspace();
    let other = fixture.home.join("workspaces/unrelated");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("notes.md"), "keep me").unwrap();
    let output = fixture.run(&["delete-instance", "review"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.home.join("workspaces/review").exists());
    assert!(!fixture.home.join("runtime/cli-test/review.json").exists());
    assert!(
        !fixture
            .home
            .join("templates/website/.tandem-cli-test-review.owner.json")
            .exists()
    );
    assert_eq!(
        fs::read_to_string(other.join("notes.md")).unwrap(),
        "keep me"
    );
}

#[test]
fn cleanup_recovery_preserves_work_when_ownership_or_docker_evidence_is_unavailable() {
    for failure in [
        "missing-receipt",
        "foreign-receipt",
        "docker-unavailable",
        "unmanaged-container",
        "foreign-workspace",
        "symlinked-workspace",
    ] {
        let fixture = damaged_workspace();
        let receipt = fixture
            .home
            .join("templates/website/.tandem-cli-test-review.owner.json");
        match failure {
            "missing-receipt" => fs::remove_file(&receipt).unwrap(),
            "foreign-receipt" => {
                let mut owner: serde_json::Value = serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
                owner["namespace"] = "other".into();
                fs::write(&receipt, owner.to_string()).unwrap();
            }
            "docker-unavailable" => fs::write(fixture.bin.join("docker"), "#!/bin/sh\nexit 99\n").unwrap(),
            "unmanaged-container" => fs::write(fixture.bin.join("docker"), "#!/bin/sh\ncase \"$*\" in *com.docker.compose.project=cli-test-review*) printf 'foreign-container\\n';; esac\n").unwrap(),
            "foreign-workspace" => {
                let path = fixture.home.join("runtime/cli-test/review.json");
                let mut record: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
                record["expected"]["workspace"] = fixture.source.display().to_string().into();
                fs::write(path, record.to_string()).unwrap();
            }
            "symlinked-workspace" => {
                let workspace = fixture.home.join("workspaces/review");
                let preserved = fixture.home.join("preserved-workspace");
                fs::rename(&workspace, &preserved).unwrap();
                std::os::unix::fs::symlink(preserved, workspace).unwrap();
            }
            _ => unreachable!(),
        }
        let output = fixture.run(&["delete-instance", "review"]);
        assert!(!output.status.success(), "{failure}");
        assert_eq!(
            fs::read_to_string(fixture.home.join("workspaces/review/notes.md")).unwrap(),
            "workspace notes"
        );
        assert!(fixture.home.join("runtime/cli-test/review.json").exists());
    }
}
