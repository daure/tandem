use super::*;

#[test]
fn action_keys_default_to_d_for_details_and_support_configurable_letters_and_enter() {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().to_path_buf(), "keys-test".into(), 9876).unwrap();
    assert!(config.keys[0].matches(KeyEvent::from(Key::Char('d'))));
    assert!(
        !config
            .keys
            .iter()
            .any(|key| key.matches(KeyEvent::from(Key::Char('v'))))
    );
    for (name, value, expected) in [
        ("INFO", "Enter", KeySpec::key(Key::Enter)),
        ("INFO", "i", KeySpec::plain('i')),
        ("REFRESH", "R", KeySpec::shifted('r')),
    ] {
        assert_eq!(action_key(name, value).unwrap(), expected);
    }
    for (name, value) in [("INFO", "ctrl+;"), ("REFRESH", "Enter"), ("INFO", "")] {
        assert!(action_key(name, value).is_err());
    }
}

#[test]
fn nested_catalogs_separate_provider_packages_and_refuse_ambiguous_instance_layouts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("templates");
    fs::create_dir_all(root.join("instances/website")).unwrap();
    fs::write(root.join("instances/website/tandem.json"), "{}").unwrap();
    fs::create_dir_all(root.join("providers/sample")).unwrap();
    fs::write(root.join("providers/sample/provider.json"), "{}").unwrap();
    let config = Config::at(directory.path().into(), "nested-test".into(), 9876).unwrap();
    assert_eq!(config.templates, root.join("instances"));
    let templates = crate::environments::templates::list(&config).unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].name, "website");
    let instructions = crate::environments::Environments::new(config)
        .instructions()
        .unwrap();
    assert_eq!(
        instructions.template_repository_root,
        root.display().to_string()
    );
    assert_eq!(
        instructions.provider_templates_root,
        root.join("providers").display().to_string()
    );
    assert_eq!(
        instructions.rule_templates_root,
        root.join("rules").display().to_string()
    );
    fs::create_dir_all(root.join("legacy")).unwrap();
    fs::write(root.join("legacy/tandem.json"), "{}").unwrap();
    assert!(
        Config::at(directory.path().into(), "mixed-test".into(), 9876)
            .err()
            .unwrap()
            .contains("both flat and nested")
    );
}

fn template_backups(home: &Path) -> Vec<PathBuf> {
    fs::read_dir(home)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("workspace-agents.template.")
                && path.extension().is_some_and(|extension| extension == "bak")
        })
        .collect()
}

#[test]
fn startup_installs_the_bundled_template_and_backs_up_a_legacy_runtime_copy() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    fs::write(
        home.join("workspace-agents.template.md"),
        "Legacy template\n",
    )
    .unwrap();
    let config = Config::at(home.into(), "upgrade-test".into(), 9876).unwrap();
    assert_eq!(
        fs::read_to_string(config.workspace_agents_template()).unwrap(),
        include_str!("../../../workspace-agents.template.md")
    );
    let backups = template_backups(home);
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read_to_string(&backups[0]).unwrap(),
        "Legacy template\n"
    );
}

#[test]
fn bundled_template_changes_refresh_the_runtime_copy_once_and_preserve_workspace_edits() {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().into(), "upgrade-test".into(), 9876).unwrap();
    fs::write(
        config.home.join(".workspace-agents.bundled.md"),
        "Previous bundled revision\n",
    )
    .unwrap();
    fs::write(config.workspace_agents_template(), "Local template edits\n").unwrap();
    let workspace = config.workspaces.join("review");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("AGENTS.md"), "Workspace edits\n").unwrap();
    fs::write(&config.instructions, "Runtime agent instructions\n").unwrap();

    Config::at(config.home.clone(), config.namespace.clone(), config.port).unwrap();
    assert_eq!(
        fs::read_to_string(config.workspace_agents_template()).unwrap(),
        include_str!("../../../workspace-agents.template.md")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("AGENTS.md")).unwrap(),
        "Workspace edits\n"
    );
    assert_eq!(
        fs::read_to_string(&config.instructions).unwrap(),
        "Runtime agent instructions\n"
    );
    let backups = template_backups(&config.home);
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read_to_string(&backups[0]).unwrap(),
        "Local template edits\n"
    );

    fs::write(
        config.workspace_agents_template(),
        "Current revision edits\n",
    )
    .unwrap();
    Config::at(config.home.clone(), config.namespace.clone(), config.port).unwrap();
    assert_eq!(
        fs::read_to_string(config.workspace_agents_template()).unwrap(),
        "Current revision edits\n"
    );
    assert_eq!(template_backups(&config.home).len(), 1);
}

#[test]
fn concurrent_startups_share_one_template_upgrade_across_namespaces() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("workspace-agents.template.md"),
        "Legacy template\n",
    )
    .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for index in 0..4 {
            let barrier = barrier.clone();
            let home = directory.path();
            workers.push(scope.spawn(move || {
                barrier.wait();
                Config::at(home.into(), format!("upgrade-{index}"), 9876).unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    assert_eq!(template_backups(directory.path()).len(), 1);
    assert_eq!(
        fs::read_to_string(directory.path().join("workspace-agents.template.md")).unwrap(),
        include_str!("../../../workspace-agents.template.md")
    );
}

#[cfg(unix)]
#[test]
fn template_upgrade_refuses_symlinks_and_preserves_their_targets() {
    for filename in [
        "workspace-agents.template.md",
        ".workspace-agents.bundled.md",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        fs::write(&outside, "Keep me\n").unwrap();
        let home = directory.path().join("home");
        fs::create_dir(&home).unwrap();
        std::os::unix::fs::symlink(&outside, home.join(filename)).unwrap();
        let result = Config::at(home.clone(), "upgrade-test".into(), 9876);
        assert!(result.err().unwrap().contains("must be a regular file"));
        assert_eq!(fs::read_to_string(outside).unwrap(), "Keep me\n");
        assert!(template_backups(&home).is_empty());
    }
}
