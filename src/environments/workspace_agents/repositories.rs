use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use serde_json::Value;

use super::{Config, Instance, Repository, code, read_text};
use crate::environments::compose;

#[derive(Deserialize)]
struct Model {
    services: BTreeMap<String, Service>,
}

#[derive(Deserialize)]
struct Service {
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    volumes: Vec<Mount>,
    working_dir: Option<String>,
    command: Option<Value>,
    entrypoint: Option<Value>,
    build: Option<Value>,
}

#[derive(Deserialize)]
struct Mount {
    #[serde(rename = "type")]
    kind: String,
    source: Option<String>,
    target: String,
}

pub(super) fn table(
    config: &Config,
    instance: &Instance,
    repositories: &[Repository],
    compose_file: &Path,
) -> Result<(String, bool), String> {
    let workspace = Path::new(&instance.workspace);
    let mut targets: BTreeSet<String> = repositories
        .iter()
        .map(|repo| repo.target.clone())
        .collect();
    for entry in fs::read_dir(workspace).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
            && entry.path().join(".git").exists()
        {
            targets.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    let services = services(config, instance, compose_file)?;
    let mut unmapped_services: BTreeSet<String> = if services.is_empty() {
        instance
            .services
            .iter()
            .map(|service| service.name.clone())
            .collect()
    } else {
        services.keys().cloned().collect()
    };
    unmapped_services.remove("repo-sync");
    if targets.is_empty() && unmapped_services.is_empty() {
        return Ok(("No services or repositories identified.".into(), false));
    }
    let mut lines = vec![
        "| Repository | Service | Code path in container | Agent guidance |".into(),
        "|---|---|---|---|".into(),
    ];
    let mut has_guidance = false;
    for target in targets {
        let root = workspace.join(&target);
        let guidance: Vec<_> = ["AGENTS.md", "agents.md"]
            .into_iter()
            .filter(|filename| root.join(filename).is_file())
            .map(|filename| cell(&format!("./{target}/{filename}")))
            .collect();
        has_guidance |= !guidance.is_empty();
        let guidance = if guidance.is_empty() {
            "None".into()
        } else {
            guidance.join(" and ")
        };
        let source = repositories
            .iter()
            .find(|repo| repo.target == target)
            .map(|repo| Path::new(&repo.source));
        let mut mappings = BTreeSet::new();
        for (name, service) in &services {
            if name == "repo-sync" {
                continue;
            }
            for path in code_paths(service, &root, source) {
                mappings.insert((cell(name), cell(&path.display().to_string())));
                unmapped_services.remove(name);
            }
        }
        if mappings.is_empty() {
            mappings.insert(("Not identified".into(), "Not identified".into()));
        }
        for (service, path) in mappings {
            lines.push(format!(
                "| {} | {service} | {path} | {guidance} |",
                cell(&format!("./{target}"))
            ));
        }
    }
    for service in unmapped_services {
        lines.push(format!("| — | {} | — | — |", cell(&service)));
    }
    Ok((lines.join("\n"), has_guidance))
}

fn services(
    config: &Config,
    instance: &Instance,
    path: &Path,
) -> Result<BTreeMap<String, Service>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err("rendered Compose must be a regular file".into());
        }
        Ok(_) => {}
    }
    let model: Model = serde_json::from_str(&read_text(path)?)
        .map_err(|_| "cannot read repository mappings from rendered Compose".to_string())?;
    for service in model.services.values() {
        for (key, expected) in [
            (compose::NAMESPACE, config.namespace.as_str()),
            (compose::KIND, "instance"),
            (compose::INSTANCE, instance.name.as_str()),
            (compose::DIRECTORY, instance.template_directory.as_str()),
            (compose::WORKSPACE, instance.workspace.as_str()),
            (compose::TEMPLATE, instance.template.as_str()),
        ] {
            if service
                .labels
                .get(key)
                .map(|value| value.replace("$$", "$"))
                != Some(expected.into())
            {
                return Err(
                    "rendered Compose repository mappings do not match this instance".into(),
                );
            }
        }
    }
    Ok(model.services)
}

fn code_paths(
    service: &Service,
    root: &Path,
    repository_source: Option<&Path>,
) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();
    for (index, mount) in service.volumes.iter().enumerate() {
        if mount.kind != "bind" {
            continue;
        }
        let Some(source) = mount.source.as_deref().and_then(absolute_path) else {
            continue;
        };
        let Some(target) = absolute_path(&mount.target) else {
            continue;
        };
        if source.is_file() {
            continue;
        }
        let direct = source.starts_with(root);
        let path = if direct {
            target.clone()
        } else if let Ok(relative) = root.strip_prefix(&source) {
            target.join(relative)
        } else {
            continue;
        };
        let shadowed = service
            .volumes
            .iter()
            .enumerate()
            .any(|(other_index, other)| {
                other_index != index
                    && absolute_path(&other.target).is_some_and(|other_target| {
                        path.starts_with(&other_target) && other_target.starts_with(&target)
                    })
            });
        if !shadowed && (direct || uses_repository(service, root, repository_source, &path)) {
            paths.insert(path);
        }
    }
    paths
}

fn uses_repository(
    service: &Service,
    root: &Path,
    source: Option<&Path>,
    container_path: &Path,
) -> bool {
    let working_dir = service.working_dir.as_deref().and_then(absolute_path);
    if working_dir
        .as_ref()
        .is_some_and(|path| path.starts_with(container_path))
    {
        return true;
    }
    let build_context = service
        .build
        .as_ref()
        .and_then(|build| build.get("context"))
        .and_then(Value::as_str)
        .and_then(absolute_path);
    if build_context.is_some_and(|path| {
        path.starts_with(root)
            || source.is_some_and(|source| source.is_absolute() && path.starts_with(source))
    }) {
        return true;
    }
    [service.command.as_ref(), service.entrypoint.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .any(|argument| {
            let argument = argument.replace("$$", "$");
            let path = Path::new(&argument);
            let resolved = if path.is_absolute() {
                Some(path.to_path_buf())
            } else {
                working_dir.as_ref().map(|directory| directory.join(path))
            };
            resolved.is_some_and(|path| {
                !path.components().any(|part| part == Component::ParentDir)
                    && path.starts_with(container_path)
            })
        })
}

fn absolute_path(value: &str) -> Option<PathBuf> {
    let path = PathBuf::from(value.replace("$$", "$"));
    (path.is_absolute() && !path.components().any(|part| part == Component::ParentDir))
        .then_some(path)
}

fn cell(value: &str) -> String {
    code(value).replace('|', "\\|")
}
