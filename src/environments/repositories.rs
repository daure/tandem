use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use super::{
    command::{Progress, remaining, run},
    config::{private_file, read_text},
};
use crate::store::environments::{Repository, RepositoryCheckout, Template};

pub(super) fn validate(repositories: &[Repository]) -> Result<(), String> {
    for (index, repository) in repositories.iter().enumerate() {
        let target = &repository.target;
        if target.is_empty()
            || target.len() > 240
            || target.split('/').any(|part| {
                part.is_empty()
                    || part.starts_with('.')
                    || !part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            })
        {
            return Err(
                "repository target must be a relative path of non-hidden directory names".into(),
            );
        }
        for other in &repositories[..index] {
            if Path::new(target).starts_with(&other.target)
                || Path::new(&other.target).starts_with(target)
            {
                return Err(format!(
                    "repository targets overlap: {target} and {}",
                    other.target
                ));
            }
        }
        let source = &repository.source;
        if source.trim() != source || source.is_empty() || source.chars().any(char::is_control) {
            return Err(format!("repository {target}: invalid source"));
        }
        if !Path::new(source).is_absolute() {
            let url = reqwest::Url::parse(source).map_err(|_| {
                format!("repository {target}: use an absolute local path, https:// or ssh:// URL")
            })?;
            if !matches!(url.scheme(), "https" | "ssh")
                || url.host_str().is_none()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || (url.scheme() == "https" && !url.username().is_empty())
            {
                return Err(format!(
                    "repository {target}: use HTTPS or SSH without embedded credentials, query or fragment"
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn prepare(
    root: &Path,
    workspace: &Path,
    template: &Template,
    branch: Option<&str>,
    deadline: Instant,
    progress: Progress,
    mut prepared: impl FnMut(RepositoryCheckout) -> Result<(), String>,
) -> Result<(), String> {
    let repositories = &template.manifest.repositories;
    validate(repositories)?;
    if repositories.is_empty() {
        return Ok(());
    }
    real_directory(root)?;
    real_directory(workspace)?;
    if workspace.parent() != Some(root) {
        return Err("repository workspace must be a direct child of the workspace root".into());
    }
    // Preflight every existing target before installing any new checkout.
    for repository in repositories {
        let target = checked_target(workspace, &repository.target, false)?;
        if fs::symlink_metadata(&target).is_ok() {
            validate_checkout(&target, repository, deadline)?;
        }
    }
    claim_workspace(workspace, template)?;
    for repository in repositories {
        remaining(deadline)?;
        let target = checked_target(workspace, &repository.target, true)?;
        if fs::symlink_metadata(&target).is_ok() {
            validate_checkout(&target, repository, deadline)?;
            progress(format!(
                "Preserving repository {} (branch and edits unchanged)",
                repository.target
            ));
            prepared(RepositoryCheckout {
                target: repository.target.clone(),
                path: target.display().to_string(),
                cloned: false,
            })?;
            continue;
        }
        progress(format!("Cloning repository {}", repository.target));
        let staging = tempfile::Builder::new()
            .prefix(".tandem-clone-")
            .tempdir_in(target.parent().ok_or("repository target has no parent")?)
            .map_err(|error| error.to_string())?;
        let checkout = staging.path().join("checkout");
        let mut command = git(staging.path());
        command
            .args([
                "clone",
                "--no-local",
                "--origin",
                "origin",
                "--template=",
                "--",
                &repository.source,
            ])
            .arg(&checkout);
        // Git can echo credential-helper output. Only expose the target and recovery action.
        run(command, remaining(deadline)?, None).map_err(|_| format!(
            "repository {}: clone failed or timed out; check source access, host Git credentials and the startup budget",
            repository.target
        ))?;
        select_branch(&checkout, branch, deadline)?;
        validate_checkout(&checkout, repository, deadline)?;
        let selected = query(&checkout, &["symbolic-ref", "--short", "HEAD"], deadline)?;
        checked_target(workspace, &repository.target, false)?;
        install(&checkout, &target)?;
        prepared(RepositoryCheckout {
            target: repository.target.clone(),
            path: target.display().to_string(),
            cloned: true,
        })?;
        progress(format!(
            "Repository {} ready on branch {selected}",
            repository.target
        ));
    }
    Ok(())
}

fn git(directory: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(directory).stdin(Stdio::null());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false")
        .env("GCM_INTERACTIVE", "Never")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -oBatchMode=yes -oStrictHostKeyChecking=yes",
        )
        .env("GIT_ALLOW_PROTOCOL", "file:https:ssh")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "submodule.recurse=false",
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
        ]);
    command
}

fn query(directory: &Path, args: &[&str], deadline: Instant) -> Result<String, String> {
    let mut command = git(directory);
    command.args(args);
    run(command, remaining(deadline)?, None)
        .map(|output| output.trim().to_owned())
        .map_err(|_| {
            "repository Git operation failed; verify the checkout and startup budget".into()
        })
}

fn select_branch(checkout: &Path, branch: Option<&str>, deadline: Instant) -> Result<(), String> {
    if let Some(branch) = branch {
        if query(checkout, &["symbolic-ref", "--short", "HEAD"], deadline)? == branch {
            return query(
                checkout,
                &["rev-parse", "--verify", "HEAD^{commit}"],
                deadline,
            )
            .map(|_| ());
        }
        let remote = format!("refs/remotes/origin/{branch}");
        let exists = query(
            checkout,
            &["for-each-ref", "--format=%(refname)", &remote],
            deadline,
        )?
        .lines()
        .any(|reference| reference == remote);
        if exists {
            query(
                checkout,
                &["switch", "--track", "-c", branch, &remote],
                deadline,
            )?;
        } else {
            query(
                checkout,
                &[
                    "switch",
                    "--no-track",
                    "-c",
                    branch,
                    "refs/remotes/origin/HEAD",
                ],
                deadline,
            )?;
        }
    }
    query(
        checkout,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        deadline,
    )?;
    Ok(())
}

fn validate_checkout(
    target: &Path,
    repository: &Repository,
    deadline: Instant,
) -> Result<(), String> {
    real_directory(target)?;
    real_directory(&target.join(".git"))?;
    let origin = query(
        target,
        &[
            "config",
            "--local",
            "--no-includes",
            "--get-all",
            "remote.origin.url",
        ],
        deadline,
    )?;
    if origin != repository.source {
        return Err(format!(
            "repository {}: origin differs from the declared source; existing work preserved",
            repository.target
        ));
    }
    let top = query(target, &["rev-parse", "--show-toplevel"], deadline)?;
    if Path::new(&top) != target {
        return Err(format!(
            "repository {}: checkout root differs from its target",
            repository.target
        ));
    }
    Ok(())
}

fn real_directory(path: &Path) -> Result<(), String> {
    if !fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_dir()
        || fs::canonicalize(path).map_err(|error| error.to_string())? != path
    {
        return Err(format!(
            "repository path must be a real directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn checked_target(
    workspace: &Path,
    relative: &str,
    create_parents: bool,
) -> Result<PathBuf, String> {
    real_directory(workspace)?;
    let mut current = workspace.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(_) => real_directory(&current)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if create_parents && index + 1 < parts.len() {
                    fs::create_dir(&current).map_err(|error| error.to_string())?;
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(current)
}

fn claim_workspace(workspace: &Path, template: &Template) -> Result<(), String> {
    let receipt = workspace.join(".tandem-repositories.json");
    let owner = serde_json::to_string(&template.directory).map_err(|error| error.to_string())?;
    match private_file(&receipt, true) {
        Ok(mut file) => file
            .write_all(owner.as_bytes())
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(&receipt)
                .map_err(|error| error.to_string())?
                .file_type()
                .is_file()
                || read_text(&receipt)? != owner
            {
                return Err("repository workspace belongs to another template or has an invalid ownership receipt".into());
            }
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn install(checkout: &Path, target: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source =
            CString::new(checkout.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
        let target =
            CString::new(target.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
        // Publish a complete checkout without replacing a concurrently created target.
        if unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (checkout, target);
        Err("atomic repository provisioning currently requires Linux".into())
    }
}

#[cfg(test)]
#[path = "tests/repositories.rs"]
mod tests;
