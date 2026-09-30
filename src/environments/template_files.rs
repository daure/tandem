use std::{
    fs,
    path::Path,
    time::{Instant, UNIX_EPOCH},
};

use super::command::{Progress, remaining};
use crate::store::environments::{Repository, Template, TemplateFile};

pub(super) fn inspect(directory: &Path) -> Result<(Option<String>, Vec<TemplateFile>), String> {
    let root = directory.join("tandem-files");
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((None, Vec::new()));
        }
        Err(error) => return Err(format!("{}: {error}", root.display())),
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err("tandem-files must be a real directory".into());
        }
        Ok(_) => {}
    }
    let mut files = Vec::new();
    collect(&root, &root, &mut files)?;
    Ok((Some(root.display().to_string()), files))
}

fn collect(root: &Path, directory: &Path, files: &mut Vec<TemplateFile>) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
        let name = relative
            .to_str()
            .ok_or("tandem-files paths must be UTF-8")?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_dir() && !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "tandem-files/{name}: only real directories and regular files are supported"
            ));
        }
        if relative.components().any(|part| part.as_os_str() == ".git")
            || relative.components().next().is_some_and(|part| {
                let name = part.as_os_str().to_string_lossy();
                name.starts_with(".tandem-") || name == "AGENTS.md"
            })
        {
            return Err(format!("tandem-files/{name}: reserved workspace path"));
        }
        files.push(TemplateFile {
            path: name.into(),
            directory: metadata.is_dir(),
            size_bytes: if metadata.is_dir() { 0 } else { metadata.len() },
            modified_at_unix_nanoseconds: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .and_then(|duration| u64::try_from(duration.as_nanos()).ok()),
        });
        if metadata.is_dir() {
            collect(root, &path, files)?;
        }
    }
    Ok(())
}

pub(super) fn validate(files: &[TemplateFile], repositories: &[Repository]) -> Result<(), String> {
    for file in files {
        let path = Path::new(&file.path);
        for repository in repositories {
            let target = Path::new(&repository.target);
            if path.starts_with(target) || !file.directory && target.starts_with(path) {
                return Err(format!(
                    "tandem-files/{} conflicts with repository target {}",
                    file.path, repository.target
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn prepare(
    workspace: &Path,
    template: &Template,
    deadline: Instant,
    progress: Progress,
) -> Result<(), String> {
    let (root, files) = inspect(Path::new(&template.directory))?;
    let Some(root) = root else { return Ok(()) };
    validate(&files, &template.manifest.repositories)?;
    // Check all destinations before copying so a bad path cannot cause a partial overlay.
    for file in &files {
        remaining(deadline)?;
        checked_destination(workspace, file)?;
    }
    progress("Copying tandem-files into workspace before repository preparation".into());
    for file in &files {
        remaining(deadline)?;
        let destination = workspace.join(&file.path);
        if checked_destination(workspace, file)? {
            continue;
        }
        let source = Path::new(&root).join(&file.path);
        let metadata = fs::symlink_metadata(&source).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink()
            || metadata.is_dir() != file.directory
            || !metadata.is_dir() && !metadata.is_file()
        {
            return Err(format!(
                "tandem-files/{} changed during preparation",
                file.path
            ));
        }
        if file.directory {
            fs::create_dir(&destination).map_err(|error| error.to_string())?;
        } else {
            let temporary = tempfile::Builder::new()
                .prefix(".tandem-copy-")
                .tempfile_in(
                    destination
                        .parent()
                        .ok_or("file destination has no parent")?,
                )
                .map_err(|error| error.to_string())?;
            fs::copy(&source, temporary.path()).map_err(|error| error.to_string())?;
            temporary
                .as_file()
                .sync_all()
                .map_err(|error| error.to_string())?;
            temporary
                .persist_noclobber(&destination)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn checked_destination(workspace: &Path, file: &TemplateFile) -> Result<bool, String> {
    let mut path = workspace.to_path_buf();
    let relative = Path::new(&file.path);
    for component in relative.components() {
        path.push(component);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let directory = path != workspace.join(relative) || file.directory;
        if metadata.file_type().is_symlink()
            || directory && !metadata.is_dir()
            || !directory && !metadata.is_file()
        {
            return Err(format!(
                "{}: tandem-files destination must be a real {}",
                path.display(),
                if directory {
                    "directory"
                } else {
                    "regular file"
                }
            ));
        }
    }
    Ok(true)
}
