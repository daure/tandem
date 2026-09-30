use super::*;
use crate::environments::{config::Config, templates};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

fn git_test(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(path)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn seed(path: &Path) {
    fs::create_dir(path).unwrap();
    git_test(path, &["init", "-b", "trunk"]);
    fs::write(path.join("file.txt"), "default content").unwrap();
    git_test(path, &["add", "."]);
    git_test(path, &["commit", "-m", "Seed fixture"]);
}

fn fixture() -> (tempfile::TempDir, Config, Template, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().join("home"), "repositories-test".into(), 9876).unwrap();
    let mut template = templates::create(&config, "website").unwrap();
    let source = home.path().join("source with spaces");
    seed(&source);
    template.manifest.repositories.push(Repository {
        source: source.display().to_string(),
        target: "src/app".into(),
    });
    let workspace = config.workspaces.join("review");
    fs::create_dir(&workspace).unwrap();
    (home, config, template, workspace)
}

fn provision(
    config: &Config,
    template: &Template,
    workspace: &Path,
    branch: Option<&str>,
) -> Result<(), String> {
    prepare(
        &config.workspaces,
        workspace,
        template,
        branch,
        Instant::now() + Duration::from_secs(15),
        Arc::new(|_| {}),
        |_| Ok(()),
    )
}

fn concurrent_fixture() -> (tempfile::TempDir, Config, Template, PathBuf) {
    let (home, config, mut template, workspace) = fixture();
    let source = template.manifest.repositories[0].source.clone();
    template.manifest.repositories = (0..6)
        .map(|index| Repository {
            source: source.clone(),
            target: format!("src/repo{index}"),
        })
        .collect();
    (home, config, template, workspace)
}

fn gated_clones() -> (Progress, mpsc::Receiver<usize>, Vec<mpsc::Sender<()>>) {
    let (started, starts) = mpsc::channel();
    let (releases, gates): (Vec<_>, Vec<_>) = (0..4)
        .map(|_| {
            let (sender, receiver) = mpsc::channel();
            (sender, Mutex::new(receiver))
        })
        .unzip();
    let progress: Progress = Arc::new(move |message| {
        if let Some(index) = message.strip_prefix("Cloning repository src/repo") {
            let index = index.parse::<usize>().unwrap();
            started.send(index).unwrap();
            if let Some(gate) = gates.get(index) {
                let _ = gate.lock().unwrap().recv();
            }
        }
    });
    (progress, starts, releases)
}

#[test]
fn repository_clones_fill_four_slots_and_replace_each_finished_clone_without_a_batch_barrier() {
    let (_home, config, template, workspace) = concurrent_fixture();
    let (progress, starts, releases) = gated_clones();
    thread::scope(|scope| {
        // Drop the gates inside the scope on assertion failure so workers can exit.
        let release_gates = releases;
        let worker = scope.spawn(|| {
            let mut checkouts = Vec::new();
            prepare(
                &config.workspaces,
                &workspace,
                &template,
                Some("review"),
                Instant::now() + Duration::from_secs(15),
                progress,
                |checkout| {
                    checkouts.push(checkout);
                    Ok(())
                },
            )
            .unwrap();
            checkouts
        });
        let mut initial: Vec<_> = (0..4)
            .map(|_| starts.recv_timeout(Duration::from_secs(3)).unwrap())
            .collect();
        initial.sort_unstable();
        assert_eq!(initial, [0, 1, 2, 3]);
        assert!(matches!(
            starts.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release_gates[1].send(()).unwrap();
        assert_eq!(starts.recv_timeout(Duration::from_secs(3)).unwrap(), 4);
        assert_eq!(starts.recv_timeout(Duration::from_secs(3)).unwrap(), 5);
        for index in [0, 2, 3] {
            release_gates[index].send(()).unwrap();
        }
        let checkouts = worker.join().unwrap();
        assert_eq!(checkouts.len(), 6);
        assert!(checkouts.iter().all(|checkout| checkout.cloned));
        for repository in &template.manifest.repositories {
            assert_eq!(
                git_test(
                    &workspace.join(&repository.target),
                    &["branch", "--show-current"]
                ),
                "review"
            );
        }
    });
}

#[test]
fn checkout_record_failure_stops_queued_clones_and_records_in_flight_successes() {
    let (_home, config, template, workspace) = concurrent_fixture();
    let (progress, starts, releases) = gated_clones();
    let (recorded, records) = mpsc::channel();
    thread::scope(|scope| {
        let release_gates = releases;
        let worker = scope.spawn(|| {
            prepare(
                &config.workspaces,
                &workspace,
                &template,
                None,
                Instant::now() + Duration::from_secs(15),
                progress,
                |checkout| {
                    let failed = checkout.target == "src/repo1";
                    recorded.send(checkout).unwrap();
                    if failed {
                        Err("checkout record failed".into())
                    } else {
                        Ok(())
                    }
                },
            )
        });
        for _ in 0..4 {
            starts.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        release_gates[1].send(()).unwrap();
        assert_eq!(
            records.recv_timeout(Duration::from_secs(3)).unwrap().target,
            "src/repo1"
        );
        assert!(matches!(
            starts.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        for index in [0, 2, 3] {
            release_gates[index].send(()).unwrap();
        }
        assert_eq!(
            worker.join().unwrap().unwrap_err(),
            "checkout record failed"
        );
        let mut completed: Vec<_> = records.try_iter().map(|checkout| checkout.target).collect();
        completed.sort();
        assert_eq!(completed, ["src/repo0", "src/repo2", "src/repo3"]);
    });
    assert!(!workspace.join("src/repo4").exists());
    assert!(!workspace.join("src/repo5").exists());
    assert!(fs::read_dir(workspace.join("src")).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tandem-clone-")
    }));
}

#[test]
fn declared_repositories_choose_default_existing_or_new_local_branches() {
    for branch in [None, Some("trunk"), Some("existing"), Some("Feature-123")] {
        let (_home, config, template, workspace) = fixture();
        let source = Path::new(&template.manifest.repositories[0].source);
        git_test(source, &["switch", "-c", "existing"]);
        fs::write(source.join("file.txt"), "existing branch content").unwrap();
        git_test(source, &["add", "."]);
        git_test(source, &["commit", "-m", "Branch fixture"]);
        git_test(source, &["switch", "trunk"]);
        provision(&config, &template, &workspace, branch).unwrap();
        let target = workspace.join("src/app");
        assert_eq!(
            git_test(&target, &["branch", "--show-current"]),
            branch.unwrap_or("trunk")
        );
        assert_eq!(
            fs::read_to_string(target.join("file.txt")).unwrap(),
            if branch == Some("existing") {
                "existing branch content"
            } else {
                "default content"
            }
        );
        assert_eq!(
            git_test(&target, &["remote", "get-url", "origin"]),
            template.manifest.repositories[0].source
        );
        if branch == Some("Feature-123") {
            assert_eq!(
                git_test(
                    &target,
                    &[
                        "for-each-ref",
                        "--format=%(upstream)",
                        "refs/heads/Feature-123"
                    ]
                ),
                ""
            );
            assert_eq!(git_test(source, &["branch", "--list", "Feature-123"]), "");
        }
    }
}

#[test]
fn retries_preserve_local_commits_branch_staged_and_untracked_edits_without_source_access() {
    let (_home, config, template, workspace) = fixture();
    provision(&config, &template, &workspace, Some("review")).unwrap();
    let target = workspace.join("src/app");
    git_test(&target, &["switch", "-c", "my-work"]);
    fs::write(target.join("file.txt"), "local commit").unwrap();
    git_test(&target, &["add", "."]);
    git_test(&target, &["commit", "-m", "Local work"]);
    let head = git_test(&target, &["rev-parse", "HEAD"]);
    fs::write(target.join("file.txt"), "staged work").unwrap();
    git_test(&target, &["add", "."]);
    fs::write(target.join("file.txt"), "working edits").unwrap();
    fs::write(target.join("notes"), "untracked").unwrap();
    fs::remove_dir_all(&template.manifest.repositories[0].source).unwrap();
    provision(&config, &template, &workspace, Some("different")).unwrap();
    assert_eq!(git_test(&target, &["branch", "--show-current"]), "my-work");
    assert_eq!(git_test(&target, &["rev-parse", "HEAD"]), head);
    assert_eq!(git_test(&target, &["show", ":file.txt"]), "staged work");
    assert_eq!(
        fs::read_to_string(target.join("file.txt")).unwrap(),
        "working edits"
    );
    assert_eq!(
        fs::read_to_string(target.join("notes")).unwrap(),
        "untracked"
    );
}

#[test]
fn failed_clone_leaves_no_partial_target_and_retry_preserves_completed_repositories() {
    let (home, config, mut template, workspace) = fixture();
    let second_source = home.path().join("second");
    template.manifest.repositories.push(Repository {
        source: second_source.display().to_string(),
        target: "second".into(),
    });
    assert!(
        provision(&config, &template, &workspace, Some("review"))
            .unwrap_err()
            .contains("clone failed")
    );
    assert!(!workspace.join("second").exists());
    assert!(fs::read_dir(&workspace).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tandem-clone-")
    }));
    fs::write(workspace.join("src/app/file.txt"), "preserved").unwrap();
    seed(&second_source);
    provision(&config, &template, &workspace, Some("review")).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.join("src/app/file.txt")).unwrap(),
        "preserved"
    );
    assert_eq!(
        git_test(&workspace.join("second"), &["branch", "--show-current"]),
        "review"
    );
}

#[test]
fn existing_targets_require_matching_origin_and_workspace_owner() {
    let (_home, config, mut template, workspace) = fixture();
    provision(&config, &template, &workspace, None).unwrap();
    template.manifest.repositories[0].source = "/different/source".into();
    assert!(
        provision(&config, &template, &workspace, None)
            .unwrap_err()
            .contains("origin differs")
    );
    template.manifest.repositories[0].source =
        git_test(&workspace.join("src/app"), &["remote", "get-url", "origin"]);
    template.directory = config.templates.join("other").display().to_string();
    assert!(
        provision(&config, &template, &workspace, None)
            .unwrap_err()
            .contains("another template")
    );
}

#[test]
fn manifests_reject_unsafe_overlapping_and_competing_repository_declarations() {
    let (_home, config, template, _workspace) = fixture();
    for target in [
        "",
        ".",
        "..",
        "../outside",
        "/outside",
        "src/../app",
        "src//app",
        ".git",
        ".tandem-data",
        "src\\app",
    ] {
        assert!(
            validate(&[Repository {
                source: "/source".into(),
                target: target.into()
            }])
            .is_err(),
            "{target}"
        );
    }
    for source in [
        "relative/path",
        "--upload-pack=evil",
        "ext::sh evil",
        "file:///source",
        "https://token@host/repo",
        "ssh://user:secret@host/repo",
        "https://host/repo?token=secret",
    ] {
        assert!(
            validate(&[Repository {
                source: source.into(),
                target: "app".into()
            }])
            .is_err(),
            "{source}"
        );
    }
    for target in ["src/app", "src", "src/app/nested"] {
        let mut repositories = template.manifest.repositories.clone();
        repositories.push(Repository {
            source: "/source".into(),
            target: target.into(),
        });
        assert!(validate(&repositories).is_err());
    }
    let mut manifest = template.manifest.clone();
    manifest.one_shots.push("repo-sync".into());
    fs::write(
        &template.manifest_file,
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    assert!(
        templates::get(&config, "website")
            .unwrap_err()
            .contains("repo-sync")
    );
}

#[cfg(unix)]
#[test]
fn provisioning_rejects_symlinked_and_occupied_targets_without_touching_them() {
    for kind in ["workspace", "parent", "target", "occupied", "git"] {
        let (home, config, template, workspace) = fixture();
        let outside = home.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "untouched").unwrap();
        match kind {
            "workspace" => {
                fs::remove_dir(&workspace).unwrap();
                std::os::unix::fs::symlink(&outside, &workspace).unwrap();
            }
            "parent" => std::os::unix::fs::symlink(&outside, workspace.join("src")).unwrap(),
            "target" => {
                fs::create_dir(workspace.join("src")).unwrap();
                std::os::unix::fs::symlink(&outside, workspace.join("src/app")).unwrap();
            }
            "git" => {
                fs::create_dir_all(workspace.join("src/app")).unwrap();
                std::os::unix::fs::symlink(
                    Path::new(&template.manifest.repositories[0].source).join(".git"),
                    workspace.join("src/app/.git"),
                )
                .unwrap();
            }
            _ => fs::create_dir_all(workspace.join("src/app")).unwrap(),
        }
        assert!(
            provision(&config, &template, &workspace, None).is_err(),
            "{kind}"
        );
        assert_eq!(
            fs::read_to_string(outside.join("keep")).unwrap(),
            "untouched"
        );
    }
}

#[test]
fn empty_sources_fail_without_installing_an_unborn_checkout() {
    for branch in [None, Some("trunk"), Some("review")] {
        let (home, config, mut template, workspace) = fixture();
        let source = home.path().join("empty");
        fs::create_dir(&source).unwrap();
        git_test(&source, &["init", "-b", "trunk"]);
        template.manifest.repositories[0].source = source.display().to_string();
        assert!(provision(&config, &template, &workspace, branch).is_err());
        assert!(!workspace.join("src/app").exists());
    }
}

#[test]
fn repository_preparation_respects_the_startup_deadline() {
    let (_home, config, template, workspace) = fixture();
    let error = prepare(
        &config.workspaces,
        &workspace,
        &template,
        Some("review"),
        Instant::now(),
        Arc::new(|_| {}),
        |_| Ok(()),
    )
    .unwrap_err();
    assert!(error.contains("deadline"));
    assert!(!workspace.join("src/app").exists());
}

#[test]
fn declared_and_legacy_templates_have_one_repository_provisioner() {
    let (_home, config, mut template, _workspace) = fixture();
    let mut model = serde_json::json!({"services": {"web": {"image": "nginx"}, "repo-sync": {"image": "alpine/git"}}});
    assert!(
        crate::environments::compose::decorate(&config, &template, "review", "", &mut model)
            .unwrap_err()
            .contains("repo-sync")
    );
    template.manifest.repositories.clear();
    template.manifest.one_shots.push("repo-sync".into());
    crate::environments::compose::decorate(&config, &template, "review", "", &mut model).unwrap();
    assert_eq!(
        model["services"]["repo-sync"]["labels"]["io.tandem.role"],
        "oneshot"
    );
}

#[test]
fn atomic_install_refuses_an_existing_empty_target() {
    let home = tempfile::tempdir().unwrap();
    let checkout = home.path().join("checkout");
    let target = home.path().join("target");
    fs::create_dir(&checkout).unwrap();
    fs::create_dir(&target).unwrap();
    assert!(install(&checkout, &target).is_err());
    assert!(checkout.exists());
}
