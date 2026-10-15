use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use super::{config::Config, journal, ownership, runtime_db};
use crate::store::environments::{validate_instance_name, validate_name};

pub(crate) fn matches(config: &Config, template: &str, name: &str, path: &Path) -> bool {
    path == config.workspaces.join(name) || path == config.workspaces.join(template).join(name)
}

pub(crate) fn resolve(config: &Config, template: &str, name: &str) -> Result<PathBuf, String> {
    validate_name(template)?;
    validate_instance_name(name)?;
    if let Some(instance) = journal::recorded(config, name)? {
        if instance.template != template {
            return Err("instance belongs to another template".into());
        }
        return Ok(instance.workspace.into());
    }
    if let Some((owner_template, path)) = ownership::workspace(config, name)? {
        if owner_template != template {
            return Err("instance belongs to another template".into());
        }
        return Ok(path);
    }
    let retained = if let Some(record) = super::startup::read(config, name)?
        && let Some(path) = record.workspace
    {
        if record.operation.template.as_deref() != Some(template)
            || !matches(config, template, name, &path)
        {
            return Err("startup workspace ownership mismatch".into());
        }
        Some(path)
    } else {
        None
    };
    let flat = config.workspaces.join(name);
    if retained.as_ref() == Some(&flat) {
        return Ok(flat);
    }
    let group = config.workspaces.join(template);
    let paths = runtime_db::workspace_paths(config)?;
    if paths.contains(&group) {
        return Err("template workspace folder is occupied by a flat instance".into());
    }
    if let Some(path) = retained {
        return Ok(path);
    }
    let flat_is_group = !paths.contains(&flat)
        && (paths
            .iter()
            .any(|path| path.parent() == Some(flat.as_path()))
            || config.templates.join(name).is_dir());
    match fs::symlink_metadata(&flat) {
        Ok(_) if !flat_is_group => Ok(flat),
        Ok(_) => Ok(group.join(name)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(group.join(name)),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn existing(config: &Config, name: &str) -> Result<PathBuf, String> {
    validate_instance_name(name)?;
    if let Some(instance) = journal::recorded(config, name)? {
        return Ok(instance.workspace.into());
    }
    if let Some((_, path)) = ownership::workspace(config, name)? {
        return Ok(path);
    }
    Ok(config.workspaces.join(name))
}

pub(super) fn validate(config: &Config, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(&config.workspaces)
        .map_err(|_| "workspace escapes the workspace root")?;
    let parts: Vec<_> = relative.components().collect();
    if !(1..=2).contains(&parts.len())
        || parts
            .iter()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("workspace escapes the workspace root".into());
    }
    if fs::canonicalize(&config.workspaces).map_err(|error| error.to_string())? != config.workspaces
    {
        return Err("workspace escapes the workspace root".into());
    }
    let mut current = config.workspaces.clone();
    for part in parts {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if !metadata.file_type().is_dir() => {
                return Err("workspace must be a real directory, not a symlink".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}
