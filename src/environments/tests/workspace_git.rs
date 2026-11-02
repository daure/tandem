use super::*;

#[test]
fn workspace_preparation_preserves_existing_git_history_configuration_and_exclusions() {
    let (_home, _config, _template, workspace) = fixture();
    git_test(&workspace, &["init", "-b", "personal"]);
    git_test(
        &workspace,
        &["commit", "--allow-empty", "-m", "Personal history"],
    );
    git_test(&workspace, &["remote", "add", "origin", "/personal/source"]);
    fs::write(workspace.join(".git/info/exclude"), "personal-cache/\n").unwrap();
    let head = git_test(&workspace, &["rev-parse", "HEAD"]);
    let configuration = fs::read(workspace.join(".git/config")).unwrap();

    prepare_workspace(&workspace, Instant::now() + Duration::from_secs(5)).unwrap();

    assert_eq!(git_test(&workspace, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read(workspace.join(".git/config")).unwrap(),
        configuration
    );
    assert_eq!(
        fs::read_to_string(workspace.join(".git/info/exclude")).unwrap(),
        "personal-cache/\n"
    );
}

#[cfg(unix)]
#[test]
fn workspace_preparation_preserves_git_files_and_symlinks() {
    for kind in ["file", "symlink", "dangling-symlink"] {
        let (home, _config, _template, workspace) = fixture();
        let git_path = workspace.join(".git");
        let contents = "gitdir: /personal/worktree\n";
        if kind == "file" {
            fs::write(&git_path, contents).unwrap();
        } else {
            let target = home.path().join("personal-git");
            if kind == "symlink" {
                fs::write(&target, contents).unwrap();
            }
            std::os::unix::fs::symlink(target, &git_path).unwrap();
        }

        prepare_workspace(&workspace, Instant::now() + Duration::from_secs(5)).unwrap();

        if kind == "dangling-symlink" {
            assert!(!git_path.exists());
            assert!(
                fs::symlink_metadata(&git_path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        } else {
            assert_eq!(fs::read_to_string(&git_path).unwrap(), contents);
        }
    }
}

#[test]
fn an_empty_clone_target_cannot_inherit_the_workspace_repository() {
    let (_home, config, template, workspace) = fixture();
    prepare_workspace(&workspace, Instant::now() + Duration::from_secs(5)).unwrap();
    fs::create_dir_all(workspace.join("src/app")).unwrap();
    fs::write(workspace.join("src/app/notes"), "unfinished checkout").unwrap();

    assert!(provision(&config, &template, &workspace, None).is_err());

    assert!(!workspace.join("src/app/.git").exists());
    assert_eq!(
        fs::read_to_string(workspace.join("src/app/notes")).unwrap(),
        "unfinished checkout"
    );
}

#[test]
fn workspace_git_initialization_respects_the_startup_deadline() {
    let (_home, _config, _template, workspace) = fixture();
    assert_eq!(
        prepare_workspace(&workspace, Instant::now()).unwrap_err(),
        "operation deadline reached"
    );
    assert!(!workspace.join(".git").exists());
}
