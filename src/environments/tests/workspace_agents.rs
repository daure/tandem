use super::*;
use crate::store::environments::InstanceService;

fn fixture() -> (tempfile::TempDir, Config, Instance) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().join("home ' space"), "test".into(), 9876).unwrap();
    let workspace = config.workspaces.join("review");
    fs::create_dir(&workspace).unwrap();
    let instance = Instance {
        name: "review".into(),
        template: "website".into(),
        template_directory: config.templates.join("website").display().to_string(),
        workspace: workspace.display().to_string(),
        project: config.project("review"),
        services: vec![
            InstanceService {
                name: "web".into(),
                image: Some("example/web:1".into()),
                port: Some(8000),
                url: Some("http://localhost:9876/review/web/".into()),
                ..Default::default()
            },
            InstanceService {
                name: "db".into(),
                image: Some("postgres:17-alpine".into()),
                ..Default::default()
            },
            InstanceService {
                name: "setup".into(),
                one_shot: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    (directory, config, instance)
}

#[test]
fn workspace_guidance_lists_declared_and_discovered_repositories_and_service_access() {
    let (_directory, config, instance) = fixture();
    let workspace = Path::new(&instance.workspace);
    fs::create_dir_all(workspace.join("nested/api/.git")).unwrap();
    fs::write(workspace.join("nested/api/AGENTS.md"), "API guidance").unwrap();
    fs::create_dir(workspace.join("web")).unwrap();
    fs::write(workspace.join("web/.git"), "gitdir: /a/worktree").unwrap();
    fs::write(workspace.join("web/agents.md"), "Web guidance").unwrap();
    fs::create_dir_all(workspace.join("plain/.git")).unwrap();
    let repositories = [Repository {
        source: "/unused".into(),
        target: "nested/api".into(),
    }];
    generate(&config, &instance, &repositories).unwrap();
    let text = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    for expected in [
        "# Workspace: review",
        "./nested/api/AGENTS.md",
        "./web/agents.md",
        "no AGENTS.md file found at repository root",
        "If a repository has `AGENTS.md` or `agents.md`, read it before working there",
        "postgres:17-alpine",
        "one-shot setup",
        "http://localhost:9876/review/web/",
        "container port 8000",
        "no host HTTP route",
        "ps --all --format json",
        "exec -T SERVICE COMMAND",
        "Run any commands/tests via Docker/Compose or call exposed URLs/ports",
        "disposable; preserve repository work",
        "logs --tail 100 SERVICE",
        "--project-name 'test-review'",
        "home '\\'' space",
        ".tandem-test-review.compose.json",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("{{"));
}

#[test]
fn edited_template_is_preserved_and_existing_workspace_guidance_is_user_owned() {
    let (_directory, config, instance) = fixture();
    fs::write(
        config.workspace_agents_template(),
        "# {{instance}}\n{{repositories}}\n",
    )
    .unwrap();
    let reopened = Config::at(config.home.clone(), config.namespace.clone(), config.port).unwrap();
    generate(&reopened, &instance, &[]).unwrap();
    let path = Path::new(&instance.workspace).join("AGENTS.md");
    assert!(fs::read_to_string(&path).unwrap().starts_with("# review\n"));
    fs::write(&path, "Local workspace guidance\n").unwrap();
    fs::write(config.workspace_agents_template(), "{{invalid}}").unwrap();
    generate(&config, &instance, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "Local workspace guidance\n"
    );
}

#[test]
fn invalid_templates_leave_no_partial_workspace_file() {
    let (_directory, config, instance) = fixture();
    for source in ["{{unknown}}", "{{instance", " \n"] {
        fs::write(config.workspace_agents_template(), source).unwrap();
        assert!(generate(&config, &instance, &[]).is_err());
        assert!(!Path::new(&instance.workspace).join("AGENTS.md").exists());
    }
}

#[test]
fn replacement_text_is_not_interpreted_as_template_syntax() {
    let (_directory, config, mut instance) = fixture();
    instance.services[0].image = Some("image:{{project}}".into());
    generate(&config, &instance, &[]).unwrap();
    let text = fs::read_to_string(Path::new(&instance.workspace).join("AGENTS.md")).unwrap();
    assert!(text.contains("image:{{project}}"));
}

#[cfg(unix)]
#[test]
fn workspace_guidance_rejects_symlinks_and_directories_without_changing_the_target() {
    let (directory, config, instance) = fixture();
    let outside = directory.path().join("outside.md");
    fs::write(&outside, "Keep me").unwrap();
    let path = Path::new(&instance.workspace).join("AGENTS.md");
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(
        generate(&config, &instance, &[])
            .unwrap_err()
            .contains("regular file")
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), "Keep me");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(
        generate(&config, &instance, &[])
            .unwrap_err()
            .contains("regular file")
    );
}
