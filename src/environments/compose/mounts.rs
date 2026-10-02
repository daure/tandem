use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

use serde_json::Value;

use crate::environments::config::Config;

const GUIDANCE: &str =
    "use TANDEM_WORKSPACE or a project-owned volume; mount template assets read-only";

pub(super) fn validate(config: &Config, model: &Value) -> Result<(), String> {
    validate_volume_backing(config, model)?;
    let services = model
        .get("services")
        .and_then(Value::as_object)
        .ok_or("Compose must declare services")?;
    for (service_name, service) in services {
        let service = service
            .as_object()
            .ok_or_else(|| format!("{service_name}: invalid Compose service"))?;
        let Some(volumes) = service.get("volumes") else {
            continue;
        };
        let volumes = volumes.as_array().ok_or_else(|| {
            mount_error(service_name, "<unknown>", "expected resolved volumes array")
        })?;
        for volume in volumes {
            let source = volume["source"].as_str().unwrap_or("<unknown>");
            let mount_type = volume["type"].as_str().ok_or_else(|| {
                mount_error(
                    service_name,
                    source,
                    "expected resolved mount object with type",
                )
            })?;
            if mount_type != "bind" {
                continue;
            }
            validate_bind(config, service_name, volume)
                .map_err(|reason| mount_error(service_name, source, &reason))?;
        }
    }
    Ok(())
}

fn mount_error(service: &str, source: &str, reason: &str) -> String {
    format!("{service}: bind source {source:?}: {reason}; {GUIDANCE}")
}

fn validate_bind(config: &Config, service: &str, volume: &Value) -> Result<(), String> {
    let source = volume["source"]
        .as_str()
        .ok_or("expected a bind source path")?;
    let read_only = match volume.get("read_only") {
        None => false,
        Some(value) => value.as_bool().ok_or("read_only must be a boolean")?,
    };
    if let Some(bind) = volume.get("bind") {
        let bind = bind.as_object().ok_or("bind options must be an object")?;
        if let Some(create_host_path) = bind.get("create_host_path") {
            create_host_path
                .as_bool()
                .ok_or("create_host_path must be a boolean")?;
        }
    }
    protect_source(config, service, Path::new(source), read_only)
}

fn protect_source(
    config: &Config,
    service: &str,
    source: &Path,
    read_only: bool,
) -> Result<(), String> {
    let (source, exists) = resolve_source(source)?;
    let template_repository = config.home.join("templates");
    let templates = fs::canonicalize(&template_repository).map_err(|error| {
        format!(
            "cannot resolve shared templates root {} for service {service}: {error}",
            template_repository.display()
        )
    })?;
    let protected = source.starts_with(&templates) || templates.starts_with(&source);
    if protected && !exists {
        return Err(format!(
            "missing source {} overlaps shared templates; host path creation could modify templates",
            source.display()
        ));
    }
    if protected && !read_only {
        return Err(format!(
            "writable source {} exposes shared templates {}",
            source.display(),
            templates.display()
        ));
    }
    Ok(())
}

fn validate_volume_backing(config: &Config, model: &Value) -> Result<(), String> {
    let Some(volumes) = model.get("volumes") else {
        return Ok(());
    };
    let volumes = volumes.as_object().ok_or("invalid resolved volumes")?;
    for (name, volume) in volumes {
        if volume["external"].as_bool() == Some(true) {
            continue;
        }
        let Some(options) = volume.get("driver_opts").and_then(Value::as_object) else {
            continue;
        };
        let flags = options
            .get("o")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .split(',')
            .collect::<Vec<_>>();
        if !flags.iter().any(|flag| matches!(*flag, "bind" | "rbind")) {
            continue;
        }
        let source = options
            .get("device")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("volume {name}: bind-backed volume requires device; {GUIDANCE}")
            })?;
        let read_only = flags.contains(&"ro") && !flags.contains(&"rw");
        protect_source(
            config,
            &format!("volume {name}"),
            Path::new(source),
            read_only,
        )
        .map_err(|reason| mount_error(&format!("volume {name}"), source, &reason))?;
    }
    Ok(())
}

pub(super) fn validate_build_caches(
    config: &Config,
    directory: &Path,
    model: &Value,
) -> Result<(), String> {
    let services = model["services"]
        .as_object()
        .ok_or("Compose must declare services")?;
    for (name, service) in services {
        let Some(caches) = service["build"].get("cache_to") else {
            continue;
        };
        for cache in caches
            .as_array()
            .ok_or("expected resolved build cache_to array")?
        {
            let cache = cache
                .as_str()
                .ok_or("expected resolved build cache_to string")?;
            let fields = cache
                .split(',')
                .filter_map(|field| field.split_once('='))
                .collect::<std::collections::BTreeMap<_, _>>();
            if fields.get("type") != Some(&"local") {
                continue;
            }
            let destination = fields
                .get("dest")
                .ok_or_else(|| format!("{name}: local build cache requires dest; {GUIDANCE}"))?;
            let source = directory.join(destination);
            protect_source(config, name, &source, false)
                .map_err(|reason| mount_error(name, destination, &reason))?;
        }
    }
    Ok(())
}

fn resolve_source(path: &Path) -> Result<(PathBuf, bool), String> {
    if !path.is_absolute() {
        return Err("resolved bind source must be an absolute path".into());
    }
    let mut resolved = PathBuf::new();
    let mut exists = true;
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                // A missing/.. traversal cannot be checked without creating the missing directory.
                if !exists {
                    return Err("cannot resolve '..' through a missing directory".into());
                }
                require_directory(&resolved)?;
                resolved.pop();
            }
            Component::Normal(name) => {
                if !exists {
                    resolved.push(name);
                    continue;
                }
                require_directory(&resolved)?;
                let candidate = resolved.join(name);
                match fs::symlink_metadata(&candidate) {
                    Ok(_) => {
                        // Resolve symlinks before interpreting the next '..' component.
                        resolved = fs::canonicalize(&candidate).map_err(|error| {
                            format!("cannot resolve {}: {error}", candidate.display())
                        })?;
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => {
                        resolved = candidate;
                        exists = false;
                    }
                    Err(error) => {
                        return Err(format!("cannot inspect {}: {error}", candidate.display()));
                    }
                }
            }
        }
    }
    // Preserve filesystem checks for trailing separators and '.' components discarded by Path.
    match fs::metadata(path) {
        Ok(_) => {}
        Err(error) if !exists && error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    }
    Ok((resolved, exists))
}

fn require_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/template_mounts.rs"]
mod tests;
