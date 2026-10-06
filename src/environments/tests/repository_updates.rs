use super::*;
use crate::store::environments::{RepositoryUpdate, RepositoryUpdateStatus};

fn update_one(workspace: &Path, template: &Template) -> RepositoryUpdate {
    update(
        workspace,
        &template.manifest.repositories,
        Instant::now() + Duration::from_secs(15),
    )
    .unwrap()
    .remove(0)
}

#[test]
fn clean_shallow_checkouts_fast_forward_to_their_configured_upstream_branch() {
    for branch in [None, Some("release")] {
        let (_home, config, template, workspace) = fixture();
        let source = Path::new(&template.manifest.repositories[0].source);
        git_test(source, &["branch", "release"]);
        provision(&config, &template, &workspace, branch).unwrap();
        let target = workspace.join("src/app");
        let before = git_test(&target, &["rev-parse", "HEAD"]);
        git_test(source, &["switch", branch.unwrap_or("trunk")]);
        for content in ["First update", "Latest update"] {
            fs::write(source.join("file.txt"), content).unwrap();
            git_test(source, &["add", "."]);
            git_test(source, &["commit", "-m", content]);
        }
        let latest = git_test(source, &["rev-parse", "HEAD"]);
        git_test(&target, &["config", "merge.autoStash", "true"]);
        git_test(
            &target,
            &[
                "config",
                &format!("branch.{}.mergeOptions", branch.unwrap_or("trunk")),
                "--squash --no-commit --autostash",
            ],
        );
        let result = update_one(&workspace, &template);
        assert_eq!(result.status, RepositoryUpdateStatus::Updated, "{result:?}");
        assert_eq!(result.branch.as_deref(), Some(branch.unwrap_or("trunk")));
        assert_eq!(result.before.as_deref(), Some(before.as_str()));
        assert_eq!(result.after.as_deref(), Some(latest.as_str()));
        assert_eq!(
            fs::read_to_string(target.join("file.txt")).unwrap(),
            "Latest update"
        );
        assert_eq!(git_test(&target, &["status", "--porcelain"]), "");
        let current = update_one(&workspace, &template);
        assert_eq!(
            current.status,
            RepositoryUpdateStatus::Current,
            "{current:?}"
        );
        assert_eq!(current.before, result.after);
        assert_eq!(current.after, result.after);
    }
}

#[test]
fn unsafe_branch_states_preserve_local_work_and_explain_why_the_checkout_is_skipped() {
    for (case, reason) in [
        ("unstaged", "changes"),
        ("staged", "changes"),
        ("untracked", "changes"),
        ("detached", "detached"),
        ("untracked-branch", "no configured upstream"),
        ("diverged", "diverged"),
        ("other-remote", "declared origin"),
        ("local-upstream", "declared origin"),
        ("rebase", "in progress"),
    ] {
        let (_home, config, template, workspace) = fixture();
        provision(&config, &template, &workspace, None).unwrap();
        let target = workspace.join("src/app");
        let source = Path::new(&template.manifest.repositories[0].source);
        match case {
            "unstaged" => fs::write(target.join("file.txt"), "local edit").unwrap(),
            "staged" => {
                fs::write(target.join("file.txt"), "local staged edit").unwrap();
                git_test(&target, &["add", "."]);
            }
            "untracked" => fs::write(target.join("notes"), "local notes").unwrap(),
            "detached" => {
                git_test(&target, &["switch", "--detach"]);
            }
            "untracked-branch" => {
                git_test(&target, &["switch", "--no-track", "-c", "work"]);
            }
            "diverged" => {
                fs::write(target.join("file.txt"), "local commit").unwrap();
                git_test(&target, &["add", "."]);
                git_test(&target, &["commit", "-m", "Local work"]);
            }
            "other-remote" => {
                git_test(
                    &target,
                    &[
                        "remote",
                        "add",
                        "other",
                        &template.manifest.repositories[0].source,
                    ],
                );
                git_test(&target, &["fetch", "other"]);
                git_test(&target, &["branch", "--set-upstream-to=other/trunk"]);
            }
            "local-upstream" => {
                git_test(&target, &["branch", "base"]);
                git_test(&target, &["branch", "--set-upstream-to=base"]);
            }
            "rebase" => fs::create_dir(target.join(".git/rebase-merge")).unwrap(),
            _ => unreachable!(),
        }
        let before = git_test(&target, &["rev-parse", "HEAD"]);
        let branch = git_test(&target, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let status = git_test(&target, &["status", "--porcelain"]);
        let contents = fs::read_to_string(target.join("file.txt")).unwrap();
        fs::write(source.join("file.txt"), "remote update").unwrap();
        git_test(source, &["add", "."]);
        git_test(source, &["commit", "-m", "Upstream work"]);
        let result = update_one(&workspace, &template);
        assert_eq!(
            result.status,
            RepositoryUpdateStatus::Skipped,
            "{case}: {result:?}"
        );
        assert!(result.reason.contains(reason), "{case}: {result:?}");
        assert_eq!(result.before.as_deref(), Some(before.as_str()));
        assert_eq!(result.after, result.before);
        assert_eq!(
            git_test(&target, &["rev-parse", "--abbrev-ref", "HEAD"]),
            branch
        );
        assert_eq!(git_test(&target, &["status", "--porcelain"]), status);
        assert_eq!(
            fs::read_to_string(target.join("file.txt")).unwrap(),
            contents
        );
        assert_eq!(git_test(&target, &["stash", "list"]), "");
    }
}

#[test]
fn local_commits_ahead_of_the_upstream_are_current_and_preserved() {
    let (_home, config, template, workspace) = fixture();
    provision(&config, &template, &workspace, None).unwrap();
    let target = workspace.join("src/app");
    git_test(&target, &["commit", "--allow-empty", "-m", "Local work"]);
    let before = git_test(&target, &["rev-parse", "HEAD"]);
    let result = update_one(&workspace, &template);
    assert_eq!(result.status, RepositoryUpdateStatus::Current, "{result:?}");
    assert!(result.reason.contains("ahead"));
    assert_eq!(result.before.as_deref(), Some(before.as_str()));
    assert_eq!(result.after, result.before);
}

#[test]
fn fast_forward_protects_ignored_local_files_from_incoming_tracked_files() {
    let (_home, config, template, workspace) = fixture();
    provision(&config, &template, &workspace, None).unwrap();
    let target = workspace.join("src/app");
    fs::create_dir_all(target.join(".git/info")).unwrap();
    fs::write(target.join(".git/info/exclude"), "local-data\n").unwrap();
    fs::write(target.join("local-data"), "precious local data").unwrap();
    let source = Path::new(&template.manifest.repositories[0].source);
    fs::write(source.join("local-data"), "incoming tracked file").unwrap();
    git_test(source, &["add", "."]);
    git_test(source, &["commit", "-m", "Track data"]);
    let result = update_one(&workspace, &template);
    assert_eq!(result.status, RepositoryUpdateStatus::Failed, "{result:?}");
    assert!(result.reason.contains("fast-forward failed"));
    assert_eq!(result.after, result.before);
    assert_eq!(
        fs::read_to_string(target.join("local-data")).unwrap(),
        "precious local data"
    );
}

#[test]
fn repository_failures_are_individual_and_never_replace_missing_or_unsafe_checkouts() {
    for case in ["missing", "symlink", "wrong-origin", "fetch-error"] {
        let (home, config, mut template, workspace) = concurrent_fixture();
        template.manifest.repositories.truncate(2);
        provision(&config, &template, &workspace, None).unwrap();
        let bad = workspace.join("src/repo0");
        let before = git_test(&bad, &["rev-parse", "HEAD"]);
        let outside = home.path().join("outside");
        seed(&outside);
        let source = Path::new(&template.manifest.repositories[0].source);
        fs::write(source.join("file.txt"), "remote update").unwrap();
        git_test(source, &["add", "."]);
        git_test(source, &["commit", "-m", "Remote work"]);
        let latest = git_test(source, &["rev-parse", "HEAD"]);
        match case {
            "missing" => fs::remove_dir_all(&bad).unwrap(),
            "symlink" => {
                fs::remove_dir_all(&bad).unwrap();
                std::os::unix::fs::symlink(&outside, &bad).unwrap();
            }
            "wrong-origin" => {
                git_test(
                    &bad,
                    &["remote", "set-url", "origin", outside.to_str().unwrap()],
                );
            }
            "fetch-error" => {
                git_test(
                    &bad,
                    &["config", "branch.trunk.merge", "refs/heads/missing"],
                );
                git_test(
                    &bad,
                    &[
                        "config",
                        "--add",
                        "remote.origin.fetch",
                        "+refs/heads/missing:refs/remotes/origin/missing",
                    ],
                );
            }
            _ => unreachable!(),
        }
        let results = update(
            &workspace,
            &template.manifest.repositories,
            Instant::now() + Duration::from_secs(15),
        )
        .unwrap();
        assert_eq!(
            results[0].status,
            RepositoryUpdateStatus::Failed,
            "{case}: {results:?}"
        );
        assert_eq!(
            results[1].status,
            RepositoryUpdateStatus::Updated,
            "{case}: {results:?}"
        );
        assert_eq!(results[1].after.as_deref(), Some(latest.as_str()));
        assert_eq!(
            fs::read_to_string(outside.join("file.txt")).unwrap(),
            "default content"
        );
        if case == "missing" {
            assert!(!bad.exists());
        }
        if matches!(case, "wrong-origin" | "fetch-error") {
            assert_eq!(git_test(&bad, &["rev-parse", "HEAD"]), before);
        }
    }
}

#[test]
fn repository_update_deadlines_return_failed_results_for_every_remaining_target() {
    let (_home, config, template, workspace) = concurrent_fixture();
    provision(&config, &template, &workspace, None).unwrap();
    let results = update(&workspace, &template.manifest.repositories, Instant::now()).unwrap();
    assert_eq!(results.len(), 6);
    assert!(
        results
            .iter()
            .all(|result| result.status == RepositoryUpdateStatus::Failed)
    );
    assert!(
        results
            .iter()
            .all(|result| result.before.is_none() && result.after.is_none())
    );
}
