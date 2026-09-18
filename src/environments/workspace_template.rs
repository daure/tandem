use std::{
    fs,
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use super::{
    command::Progress,
    config::{Config, read_text},
    gateway,
};

const BUNDLED: &str = include_str!("../../workspace-agents.template.md");

pub(super) fn install(config: &Config) -> Result<(), String> {
    let progress: Progress = Arc::new(|_| {});
    let _lock = gateway::lock_until(
        config,
        "workspace-agents-template",
        Instant::now() + Duration::from_secs(10),
        &progress,
    )?;
    let path = config.workspace_agents_template();
    let revision = config.home.join(".workspace-agents.bundled.md");
    let previous = read_optional(&revision)?;
    let current = read_optional(&path)?;
    if previous.as_deref() == Some(BUNDLED) && current.is_some() {
        return Ok(());
    }

    if current.as_deref() != Some(BUNDLED) {
        if let Some(current) = current {
            write_temporary(&config.home, &current, "workspace-agents.template.", ".bak")?
                .keep()
                .map_err(|error| format!("cannot back up workspace agents template: {error}"))?;
            sync_home(config)?;
        }
        replace(&path, BUNDLED)?;
        sync_home(config)?;
    }
    // Record the bundled revision only after its runtime copy is safely installed.
    replace(&revision, BUNDLED)?;
    sync_home(config)
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => read_text(path).map(Some),
        Ok(_) => Err(format!("{} must be a regular file", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn replace(path: &Path, content: &str) -> Result<(), String> {
    write_temporary(
        path.parent().ok_or("template path has no parent")?,
        content,
        ".workspace-agents-",
        ".tmp",
    )?
    .persist(path)
    .map(|_| ())
    .map_err(|error| format!("cannot install {}: {error}", path.display()))
}

fn write_temporary(
    directory: &Path,
    content: &str,
    prefix: &str,
    suffix: &str,
) -> Result<tempfile::NamedTempFile, String> {
    let mut file = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(suffix)
        .tempfile_in(directory)
        .map_err(|error| error.to_string())?;
    file.write_all(content.as_bytes())
        .map_err(|error| error.to_string())?;
    file.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    Ok(file)
}

fn sync_home(config: &Config) -> Result<(), String> {
    fs::File::open(&config.home)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("cannot sync template installation: {error}"))
}
