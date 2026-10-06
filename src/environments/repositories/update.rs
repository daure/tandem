use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use super::{checked_target, git, query, remaining, run, validate, validate_checkout};
use crate::store::environments::{Repository, RepositoryUpdate, RepositoryUpdateStatus};

pub(in crate::environments) fn update(
    workspace: &Path,
    repositories: &[Repository],
    deadline: Instant,
) -> Result<Vec<RepositoryUpdate>, String> {
    validate(repositories)?;
    Ok(repositories
        .iter()
        .map(|repository| {
            let deadline = deadline.min(Instant::now() + Duration::from_secs(60));
            let mut result = RepositoryUpdate {
                target: repository.target.clone(),
                branch: None,
                upstream: None,
                before: None,
                after: None,
                status: RepositoryUpdateStatus::Failed,
                reason: String::new(),
            };
            match checked_target(workspace, &repository.target, false) {
                Ok(target) => {
                    if let Err(error) = update_checkout(&target, repository, deadline, &mut result) {
                        result.status = RepositoryUpdateStatus::Failed;
                        result.reason = error;
                    }
                    if result.before.is_some() {
                        result.after = query(
                            &target,
                            &["rev-parse", "--verify", "HEAD^{commit}"],
                            Instant::now() + Duration::from_secs(5),
                        )
                        .ok();
                        if result.after.is_none() {
                            result.status = RepositoryUpdateStatus::Failed;
                            result.reason = "cannot verify the final revision; inspect the checkout before retrying".into();
                        }
                    }
                }
                Err(error) => result.reason = error,
            }
            result
        })
        .collect())
}

fn update_checkout(
    target: &Path,
    repository: &Repository,
    deadline: Instant,
    result: &mut RepositoryUpdate,
) -> Result<(), String> {
    validate_checkout(target, repository, deadline)?;
    let before = query(
        target,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        deadline,
    )?;
    result.before = Some(before.clone());
    let branch = query(target, &["rev-parse", "--abbrev-ref", "HEAD"], deadline)?;
    if branch == "HEAD" {
        return skip(
            result,
            "detached HEAD; select a tracking branch before updating",
        );
    }
    result.branch = Some(branch.clone());
    if let Some(reason) = blocked_checkout(target, deadline)? {
        return skip(result, &reason);
    }
    let branch_ref = format!("refs/heads/{branch}");
    let tracking = query(
        target,
        &[
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream)",
            &branch_ref,
        ],
        deadline,
    )?;
    let fields: Vec<_> = tracking.split('\0').collect();
    let [remote, remote_ref, upstream] = fields.as_slice() else {
        return skip(
            result,
            "no configured upstream; configure branch tracking before updating",
        );
    };
    if upstream.is_empty() {
        return skip(
            result,
            "no configured upstream; configure branch tracking before updating",
        );
    }
    result.upstream = Some((*upstream).into());
    if *remote != "origin"
        || !remote_ref.starts_with("refs/heads/")
        || !upstream.starts_with("refs/remotes/origin/")
    {
        return skip(
            result,
            "upstream must track a branch on the declared origin",
        );
    }
    let refspec = format!("+{remote_ref}:{upstream}");
    query(
        target,
        &[
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "origin",
            &refspec,
        ],
        deadline,
    )
    .map_err(|_| "fetch failed or timed out; check host Git credentials, source access and the upstream branch")?;
    let latest = query(
        target,
        &["rev-parse", "--verify", &format!("{upstream}^{{commit}}")],
        deadline,
    )?;
    let counts = query(
        target,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{before}...{latest}"),
        ],
        deadline,
    )?;
    let counts: Vec<_> = counts.split_whitespace().collect();
    let [ahead, behind] = counts.as_slice() else {
        return Err("cannot determine the relationship to the upstream revision".into());
    };
    if *ahead != "0" && *behind != "0" {
        return skip(
            result,
            "branch has diverged from its upstream; reconcile it manually",
        );
    }
    if *behind == "0" {
        result.status = RepositoryUpdateStatus::Current;
        result.reason = if *ahead == "0" {
            "branch matches its upstream"
        } else {
            "local commits are ahead of the upstream; preserved unchanged"
        }
        .into();
        return Ok(());
    }
    if query(
        target,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        deadline,
    )? != before
        || query(target, &["rev-parse", "--abbrev-ref", "HEAD"], deadline)? != branch
    {
        return skip(
            result,
            "checkout changed during fetch; coordinate with other sessions and retry",
        );
    }
    if let Some(reason) = blocked_checkout(target, deadline)? {
        return skip(result, &reason);
    }
    let mut command = git(target);
    command.args([
        "-c",
        &format!("branch.{branch}.mergeOptions="),
        "-c",
        "merge.autoStash=false",
        "merge",
        "--ff-only",
        "--no-autostash",
        "--no-edit",
        "--no-stat",
        "--no-overwrite-ignore",
        &latest,
    ]);
    run(command, remaining(deadline)?, None)
        .map_err(|_| "fast-forward failed or timed out; inspect the checkout and local files before retrying")?;
    if query(
        target,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        deadline,
    )? != latest
    {
        return Err("final revision differs from the fetched upstream; inspect concurrent workspace activity".into());
    }
    result.status = RepositoryUpdateStatus::Updated;
    result.reason = "fast-forwarded to the fetched upstream revision".into();
    Ok(())
}

fn skip(result: &mut RepositoryUpdate, reason: &str) -> Result<(), String> {
    result.status = RepositoryUpdateStatus::Skipped;
    result.reason = reason.into();
    Ok(())
}

fn blocked_checkout(target: &Path, deadline: Instant) -> Result<Option<String>, String> {
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
        "BISECT_LOG",
    ] {
        match fs::symlink_metadata(target.join(".git").join(marker)) {
            Ok(_) => {
                return Ok(Some(
                    "a Git operation is in progress; finish it before updating".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let status = query(
        target,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
        deadline,
    )?;
    Ok((!status.is_empty()).then(|| "checkout has staged, unstaged, untracked or submodule changes; preserve them before updating".into()))
}
