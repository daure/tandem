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
    let (_directory, config, mut instance) = fixture();
    instance.services.push(instance.services[0].clone());
    instance.services.push(InstanceService {
        name: "repo-sync".into(),
        ..Default::default()
    });
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
        "# review\n",
        "This workspace contains application repositories and Docker Compose development services.",
        "./nested/api/AGENTS.md",
        "./web/agents.md",
        "| Repository | Service | Code path in container | Agent guidance |",
        "| `./plain` | Not identified | Not identified | None |",
        "| — | `db` | — | — |",
        "| — | `setup` | — | — |",
        "| — | `web` | — | — |",
        "Read the agents.md files listed above before starting any work.\nEdit source files and run Git commands in the checked-out repositories.",
        "Compose project: `test-review`",
        "Use `docker compose -p 'test-review'` with service names for `exec` and `logs`;",
        "docker compose -p 'test-review' exec -T -w CODE_PATH SERVICE COMMAND",
        "use `docker compose -p 'test-review' ps --all` when container discovery or status is needed.",
        "For replicated services, use `exec --index N` to select a replica.",
        "Building or recreating services also requires the instance's rendered Compose configuration.",
        "Run tools and tests locally when their dependencies are available; use the service containers when commands need the environment’s runtime or dependencies.",
        "Use the exposed URLs for API and browser testing against the running application.",
        "This Docker environment supports a self-evaluation loop for code changes: exercise the application running in its containers, inspect logs and database state, then use the results to refine and recheck the changes.",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert_eq!(
        text.split_once("## HTTP URLs\n\n").unwrap().1,
        "- `web`: `http://localhost:9876/review/web/`\n\nThese URLs use a shared gateway; container ports are internal.\n"
    );
    assert!(!text.contains("{{"));
    assert!(!text.contains("`repo-sync`"));
    assert_eq!(text.matches("| — | `web` | — | — |").count(), 1);
}

#[test]
fn workspace_guidance_omits_http_content_when_no_service_has_a_url() {
    let (_directory, config, mut instance) = fixture();
    for service in &mut instance.services {
        service.url = None;
    }
    generate(&config, &instance, &[]).unwrap();
    let text = fs::read_to_string(Path::new(&instance.workspace).join("AGENTS.md")).unwrap();
    assert!(!text.contains("## HTTP URLs"));
    assert!(!text.contains("Use the exposed URLs"));
    assert!(!text.contains("These URLs use a shared gateway"));
    assert!(text.contains("## Services"));
    assert!(text.contains("| — | `db` | — | — |"));
    assert!(text.contains("docker compose -p 'test-review' exec -T -w CODE_PATH SERVICE COMMAND"));
    assert!(
        text.contains("dependencies.\n\nThis Docker environment supports a self-evaluation loop")
    );
    assert!(text.ends_with("Building or recreating services also requires the instance's rendered Compose configuration.\n"));
}

#[test]
fn workspace_guidance_omits_agent_instruction_when_no_repository_has_guidance() {
    let (_directory, config, instance) = fixture();
    let workspace = Path::new(&instance.workspace);
    fs::create_dir_all(workspace.join("plain/.git")).unwrap();
    generate(&config, &instance, &[]).unwrap();
    let text = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(!text.contains("Read the agents.md files listed above before starting any work."));
    assert!(text.contains("Code paths in containers can differ from their configured working directories.\n\nEdit source files and run Git commands in the checked-out repositories."));
}

fn rendered_compose(
    config: &Config,
    instance: &Instance,
    services: serde_json::Value,
) -> std::path::PathBuf {
    let mut template = templates::create(config, &instance.template).unwrap();
    template.manifest = Default::default();
    let mut model = serde_json::json!({"services": services});
    crate::environments::compose::decorate(config, &template, &instance.name, &mut model).unwrap();
    let path = Path::new(&instance.template_directory).join(format!(
        ".tandem-{}-{}.compose.json",
        config.namespace, instance.name
    ));
    fs::write(
        &path,
        serde_json::to_string(&model).unwrap().replace('$', "$$"),
    )
    .unwrap();
    path
}

#[test]
fn repository_table_matches_shared_workspace_mounts_using_runtime_and_build_paths() {
    let (_directory, config, instance) = fixture();
    let workspace = Path::new(&instance.workspace);
    fs::create_dir_all(workspace.join("api/.git")).unwrap();
    fs::create_dir_all(workspace.join("ui/.git")).unwrap();
    fs::write(workspace.join("api/AGENTS.md"), "API guidance").unwrap();
    let bind =
        serde_json::json!({"type": "bind", "source": instance.workspace, "target": "/workspace"});
    rendered_compose(
        &config,
        &instance,
        serde_json::json!({
            "api": {"volumes": [bind.clone()], "command": ["python", "/workspace/api/server.py"]},
            "web": {"volumes": [bind.clone()], "working_dir": "/workspace/ui"},
            "migrate": {"volumes": [bind.clone()], "build": {"context": "/sources/api/migrations"}},
            "repo-sync": {"volumes": [bind.clone()], "working_dir": "/workspace/api"},
            "other": {"volumes": [bind.clone()], "command": ["python", "/workspace/api-extra/server.py"]},
            "ambiguous": {"volumes": [bind], "working_dir": "/workspace"},
            "db": {"volumes": [{"type": "volume", "source": "data", "target": "/var/lib/db"}]}
        }),
    );
    generate(
        &config,
        &instance,
        &[
            Repository {
                source: "/sources/api".into(),
                target: "api".into(),
            },
            Repository {
                source: "https://example.invalid/ui.git".into(),
                target: "ui".into(),
            },
        ],
    )
    .unwrap();
    let text = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    let table = text
        .split_once("## Services\n\n")
        .unwrap()
        .1
        .split("\n\n")
        .next()
        .unwrap();
    assert_eq!(
        table,
        concat!(
            "| Repository | Service | Code path in container | Agent guidance |\n",
            "|---|---|---|---|\n",
            "| `./api` | `api` | `/workspace/api` | `./api/AGENTS.md` |\n",
            "| `./api` | `migrate` | `/workspace/api` | `./api/AGENTS.md` |\n",
            "| `./ui` | `web` | `/workspace/ui` | None |\n",
            "| — | `ambiguous` | — | — |\n",
            "| — | `db` | — | — |\n",
            "| — | `other` | — | — |"
        )
    );
}

#[test]
fn repository_table_handles_partial_mounts_shadowing_and_markdown_characters() {
    let (_directory, config, instance) = fixture();
    let workspace = Path::new(&instance.workspace);
    let repo = workspace.join("mono|repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir(repo.join("backend")).unwrap();
    fs::write(repo.join("AGENTS.md"), "One").unwrap();
    fs::write(repo.join("agents.md"), "Two").unwrap();
    rendered_compose(
        &config,
        &instance,
        serde_json::json!({
            "api": {"volumes": [{"type": "bind", "source": repo.join("backend"), "target": "/code/$api|v1"}]},
            "web": {"volumes": [{"type": "bind", "source": repo, "target": "/code"}]},
            "file-only": {"volumes": [{"type": "bind", "source": repo.join("AGENTS.md"), "target": "/docs/AGENTS.md"}]},
            "shadowed": {"volumes": [
                {"type": "bind", "source": instance.workspace, "target": "/workspace"},
                {"type": "volume", "source": "replacement", "target": "/workspace/mono|repo"}
            ], "working_dir": "/workspace/mono|repo"}
        }),
    );
    generate(&config, &instance, &[]).unwrap();
    let text = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    let table = text
        .split_once("## Services\n\n")
        .unwrap()
        .1
        .split("\n\n")
        .next()
        .unwrap();
    assert_eq!(
        table,
        concat!(
            "| Repository | Service | Code path in container | Agent guidance |\n",
            "|---|---|---|---|\n",
            "| `./mono\\|repo` | `api` | `/code/$api\\|v1` | `./mono\\|repo/AGENTS.md` and `./mono\\|repo/agents.md` |\n",
            "| `./mono\\|repo` | `web` | `/code` | `./mono\\|repo/AGENTS.md` and `./mono\\|repo/agents.md` |\n",
            "| — | `file-only` | — | — |\n",
            "| — | `shadowed` | — | — |"
        )
    );
}

#[test]
fn repository_table_marks_unproven_service_mappings_without_guessing() {
    let (_directory, config, instance) = fixture();
    let workspace = Path::new(&instance.workspace);
    fs::create_dir_all(workspace.join("app/.git")).unwrap();
    rendered_compose(
        &config,
        &instance,
        serde_json::json!({
            "api": {"volumes": [{"type": "bind", "source": instance.workspace, "target": "/workspace"}]},
            "image-only": {"build": {"context": workspace.join("app")}}
        }),
    );
    generate(&config, &instance, &[]).unwrap();
    let text = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(text.contains("| `./app` | Not identified | Not identified | None |"));
    assert!(text.contains("| — | `api` | — | — |"));
    assert!(text.contains("| — | `image-only` | — | — |"));
}

#[test]
fn services_table_includes_infrastructure_and_setup_without_repositories() {
    let (_directory, config, mut instance) = fixture();
    instance.services.clear();
    rendered_compose(
        &config,
        &instance,
        serde_json::json!({
            "db": {"image": "postgres:17"},
            "redis": {"image": "redis:7"},
            "migrate": {"image": "example/migrate"},
            "worker": {"image": "example/worker", "deploy": {"replicas": 0}},
            "repo-sync": {"image": "alpine/git"},
            "repo-sync-helper": {"image": "example/helper"}
        }),
    );
    generate(&config, &instance, &[]).unwrap();
    let text = fs::read_to_string(Path::new(&instance.workspace).join("AGENTS.md")).unwrap();
    let table = text
        .split_once("## Services\n\n")
        .unwrap()
        .1
        .split("\n\n")
        .next()
        .unwrap();
    assert_eq!(
        table,
        concat!(
            "| Repository | Service | Code path in container | Agent guidance |\n",
            "|---|---|---|---|\n",
            "| — | `db` | — | — |\n",
            "| — | `migrate` | — | — |\n",
            "| — | `redis` | — | — |\n",
            "| — | `repo-sync-helper` | — | — |\n",
            "| — | `worker` | — | — |"
        )
    );
    assert!(!text.contains("Read the agents.md files listed above before starting any work."));
}

#[test]
fn services_table_reports_when_no_services_or_repositories_are_identified() {
    let (_directory, config, mut instance) = fixture();
    instance.services.clear();
    generate(&config, &instance, &[]).unwrap();
    let text = fs::read_to_string(Path::new(&instance.workspace).join("AGENTS.md")).unwrap();
    assert!(text.contains("## Services\n\nNo services or repositories identified."));
}

#[test]
fn repository_mappings_reject_another_instances_compose_artifact() {
    let (_directory, config, instance) = fixture();
    let workspace = Path::new(&instance.workspace);
    fs::create_dir_all(workspace.join("app/.git")).unwrap();
    let path = rendered_compose(&config, &instance, serde_json::json!({"api": {}}));
    let mut model: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    model["services"]["api"]["labels"]["io.tandem.instance"] = "other".into();
    fs::write(&path, serde_json::to_string(&model).unwrap()).unwrap();
    assert_eq!(
        generate(&config, &instance, &[]).unwrap_err(),
        "rendered Compose repository mappings do not match this instance"
    );
    assert!(!workspace.join("AGENTS.md").exists());
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
fn template_guidance_is_appended_verbatim_after_rendering() {
    let (_directory, config, instance) = fixture();
    let template = templates::create(&config, "website").unwrap();
    let guidance = "## Project workflow\n\nKeep {{literal}} and {{instance}} unchanged.\n";
    fs::write(&template.guidance_file, guidance).unwrap();
    let path = Path::new(&instance.workspace).join("AGENTS.md");
    for source in ["# {{instance}}", "# {{instance}}\n"] {
        fs::write(config.workspace_agents_template(), source).unwrap();
        generate(&config, &instance, &[]).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("# review\n\n{guidance}")
        );
        fs::remove_file(&path).unwrap();
    }
    fs::write(&template.guidance_file, "").unwrap();
    generate(&config, &instance, &[]).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "# review\n");
    fs::write(&template.guidance_file, "Updated guidance").unwrap();
    generate(&config, &instance, &[]).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "# review\n");
}

#[test]
fn invalid_template_guidance_leaves_no_partial_workspace_file() {
    let (_directory, config, instance) = fixture();
    let template = templates::create(&config, "website").unwrap();
    fs::write(&template.guidance_file, [0xff]).unwrap();
    assert!(
        generate(&config, &instance, &[])
            .unwrap_err()
            .contains("tandem-agents.md")
    );
    assert!(!Path::new(&instance.workspace).join("AGENTS.md").exists());
}

#[test]
fn replacement_text_is_not_interpreted_as_template_syntax() {
    let (_directory, config, mut instance) = fixture();
    instance.services[0].url = Some("http://localhost/{{project}}/".into());
    generate(&config, &instance, &[]).unwrap();
    let text = fs::read_to_string(Path::new(&instance.workspace).join("AGENTS.md")).unwrap();
    assert!(text.contains("http://localhost/{{project}}/"));
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
