use super::*;

fn repository_fixture() -> Fixture {
    let fixture = Fixture::new();
    fs::remove_file(fixture.home.join("templates/website/compose.yaml")).unwrap();
    fs::write(
        fixture.bin.join("docker"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$TANDEM_HOME/docker-calls\"\nexit 99\n",
    )
    .unwrap();
    fixture
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn repository_only_creation_reopens_and_deletes_across_processes_without_docker() {
    let fixture = repository_fixture();
    fixture.save_command(OPEN);
    success(fixture.run(&["new-instance", "review", "-t", "website", "-oc"]));
    let workspace = fixture.home.join("workspaces/review");
    let guidance = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(guidance.contains("## Repositories"));
    assert!(guidance.contains("./app/AGENTS.md"));
    assert!(!guidance.contains("Docker"));
    assert!(!guidance.contains("container"));
    assert!(!guidance.contains("## HTTP"));
    assert_eq!(
        git(&workspace.join("app"), &["branch", "--show-current"]),
        "review"
    );
    let record_path = fixture.home.join("runtime/cli-test/review.json");
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(record["repositories"][0]["cloned"], true);
    assert_eq!(
        record["repositories"][0]["path"],
        workspace.join("app").display().to_string()
    );
    assert_eq!(record["expected"]["workspace_only"], true);
    fs::write(workspace.join("app/file.txt"), "local edits").unwrap();
    fs::write(workspace.join("AGENTS.md"), "User guidance\n").unwrap();
    let text = success(fixture.run(&["new-instance", "review", "-t", "website"]));
    assert!(text.contains("already exists"), "{text}");
    assert_eq!(
        fs::read_to_string(workspace.join("app/file.txt")).unwrap(),
        "local edits"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("AGENTS.md")).unwrap(),
        "User guidance\n"
    );
    success(fixture.run(&["delete-instance", "review"]));
    assert!(!workspace.exists());
    assert!(!record_path.exists());
    assert!(!fixture.home.join("docker-calls").exists());
}

#[test]
fn partial_repository_setup_is_retained_and_retry_preserves_work() {
    let fixture = repository_fixture();
    let manifest = fixture.home.join("templates/website/tandem.json");
    fs::write(
        &manifest,
        json!({"repositories": [
            {"source": fixture.source, "target": "app"},
            {"source": fixture.home.join("missing"), "target": "second"}
        ]})
        .to_string(),
    )
    .unwrap();
    assert!(
        !fixture
            .run(&["new-instance", "review", "-t", "website"])
            .status
            .success()
    );
    let record_path = fixture.home.join("runtime/cli-test/review.json");
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(record["repositories"].as_array().unwrap().len(), 1);
    assert_eq!(record["expected"]["runtime"]["workspace_ready"], false);
    assert!(record["activity"]["error"].is_string());
    let checkout = fixture.home.join("workspaces/review/app");
    fs::write(checkout.join("file.txt"), "keep changes").unwrap();
    fs::write(
        &manifest,
        json!({"repositories": [
            {"source": fixture.source, "target": "app"},
            {"source": fixture.source, "target": "second"}
        ]})
        .to_string(),
    )
    .unwrap();
    success(fixture.run(&["new-instance", "review", "-t", "website"]));
    assert_eq!(
        fs::read_to_string(checkout.join("file.txt")).unwrap(),
        "keep changes"
    );
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(record["repositories"][0]["cloned"], false);
    assert_eq!(record["repositories"][1]["cloned"], true);
    assert_eq!(record["expected"]["runtime"]["workspace_ready"], true);
    assert!(!fixture.home.join("docker-calls").exists());
}

#[test]
fn compose_only_instances_generate_service_guidance_without_repository_or_http_claims() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.home.join("templates/website/tandem.json")).unwrap();
    success(fixture.run(&["new-instance", "review", "-t", "website"]));
    let guidance = fs::read_to_string(fixture.home.join("workspaces/review/AGENTS.md")).unwrap();
    assert!(guidance.contains("## Services"));
    assert!(guidance.contains("## Docker"));
    assert!(!guidance.contains("application repositories"));
    assert!(!guidance.contains("Edit source files"));
    assert!(!guidance.contains("## HTTP URLs"));
}

#[test]
fn blank_workspaces_create_reopen_and_delete_without_git_or_docker() {
    let fixture = repository_fixture();
    let template = fixture.home.join("templates/website");
    fs::write(template.join("tandem.json"), "{}\n").unwrap();
    fs::write(
        fixture.bin.join("git"),
        "#!/bin/sh\nprintf invoked > \"$TANDEM_HOME/git-called\"\nexit 99\n",
    )
    .unwrap();
    fs::set_permissions(fixture.bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    fixture.save_command("test -s AGENTS.md && printf opened > \"$TANDEM_HOME/opened\"");

    success(fixture.run(&["new-instance", "scratch", "-t", "website", "-oc"]));
    let workspace = fixture.home.join("workspaces/scratch");
    assert_eq!(fs::read_dir(&workspace).unwrap().count(), 1);
    let guidance = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(guidance.starts_with("# scratch\n"));
    for section in ["## Repositories", "## Services", "## Docker", "## HTTP"] {
        assert!(!guidance.contains(section), "{guidance}");
    }
    let record_path = fixture.home.join("runtime/cli-test/scratch.json");
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    assert_eq!(record["expected"]["workspace_only"], true);
    assert_eq!(record["expected"]["runtime"]["workspace_ready"], true);
    fs::write(workspace.join("notes.md"), "Keep my notes").unwrap();
    success(fixture.run(&["new-instance", "scratch", "-t", "website", "-oc"]));
    assert_eq!(
        fs::read_to_string(workspace.join("notes.md")).unwrap(),
        "Keep my notes"
    );
    assert!(fixture.home.join("opened").exists());
    success(fixture.run(&["delete-instance", "scratch"]));
    assert!(!workspace.exists());
    assert!(!record_path.exists());
    assert!(!fixture.home.join("git-called").exists());
    assert!(!fixture.home.join("docker-calls").exists());
}

#[test]
fn guidance_only_workspaces_create_open_and_delete_without_git_or_docker() {
    let fixture = repository_fixture();
    let template = fixture.home.join("templates/website");
    fs::remove_file(template.join("tandem.json")).unwrap();
    let guidance =
        "## Research workspace\n\nRecord findings in notes.md. Keep {{literal}} intact.\n";
    fs::write(template.join("tandem-agents.md"), guidance).unwrap();
    fs::write(
        fixture.bin.join("git"),
        "#!/bin/sh\nprintf invoked > \"$TANDEM_HOME/git-called\"\nexit 99\n",
    )
    .unwrap();
    fs::set_permissions(fixture.bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    fixture.save_command("test -s AGENTS.md && printf opened > \"$TANDEM_HOME/opened\"");
    success(fixture.run(&["new-instance", "review", "-t", "website", "-oc"]));
    let workspace = fixture.home.join("workspaces/review");
    let generated = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(generated.starts_with("# review\n"));
    assert!(generated.ends_with(guidance));
    for unwanted in [
        "Docker",
        "## Services",
        "## Repositories",
        "## HTTP",
        "No services or repositories identified",
    ] {
        assert!(!generated.contains(unwanted), "{generated}");
    }
    assert_eq!(fs::read_dir(&workspace).unwrap().count(), 1);
    fs::write(workspace.join("notes.md"), "User work").unwrap();
    fs::write(workspace.join("AGENTS.md"), "Custom instructions\n").unwrap();
    fs::write(
        template.join("tandem-agents.md"),
        "Updated template instructions",
    )
    .unwrap();
    success(fixture.run(&["new-instance", "review", "-t", "website", "-oc"]));
    assert_eq!(
        fs::read_to_string(workspace.join("AGENTS.md")).unwrap(),
        "Custom instructions\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("notes.md")).unwrap(),
        "User work"
    );
    assert!(fixture.home.join("opened").exists());
    success(fixture.run(&["delete-instance", "review"]));
    assert!(!workspace.exists());
    assert!(!fixture.home.join("git-called").exists());
    assert!(!fixture.home.join("docker-calls").exists());
}
