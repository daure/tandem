use std::fs;

use serde_json::Value;

use crate::{
    environments::{
        compose,
        config::{Config, read_text},
    },
    store::environments::{
        ContainerState, Instance, InstanceService, ServiceRuntime, validate_name,
    },
};

pub(super) fn read(config: &Config, observed: &Instance) -> Result<Option<Instance>, String> {
    validate_name(&observed.template)?;
    let directory = config.templates.join(&observed.template);
    if directory.to_string_lossy() != observed.template_directory {
        return Ok(None);
    }
    let path = directory.join(format!(
        ".tandem-{}-{}.compose.json",
        config.namespace, observed.name
    ));
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.is_file() => {
            return Err("rendered launch topology must be a regular file".into());
        }
        Ok(_) => {}
    }
    if !fs::canonicalize(&path)
        .map_err(|error| error.to_string())?
        .starts_with(&config.templates)
    {
        return Err("rendered launch topology escapes template root".into());
    }
    let model: Value =
        serde_json::from_str(&read_text(&path)?).map_err(|error| error.to_string())?;
    if model["name"]
        .as_str()
        .is_some_and(|name| name != observed.project)
    {
        return Err("rendered launch topology project mismatch".into());
    }
    let mut expected = observed.clone();
    expected.services.clear();
    for (name, service) in model["services"]
        .as_object()
        .ok_or("rendered launch topology has no services")?
    {
        let labels = &service["labels"];
        if labels[compose::INSTANCE]
            .as_str()
            .is_some_and(|name| name != observed.name)
        {
            return Err("rendered launch topology instance mismatch".into());
        }
        for (key, value) in [
            (compose::NAMESPACE, config.namespace.as_str()),
            (compose::KIND, "instance"),
            (compose::TEMPLATE, observed.template.as_str()),
            (compose::DIRECTORY, observed.template_directory.as_str()),
            (compose::WORKSPACE, observed.workspace.as_str()),
        ] {
            if labels[key].as_str().map(|label| label.replace("$$", "$")) != Some(value.into()) {
                return Err("rendered launch topology ownership mismatch".into());
            }
        }
        let replicas = service["scale"]
            .as_u64()
            .or_else(|| service["deploy"]["replicas"].as_u64())
            .unwrap_or(1);
        if replicas > 4096 || expected.services.len().saturating_add(replicas as usize) > 4096 {
            return Err("rendered launch topology exceeds 4096 containers".into());
        }
        for replica in 1..=replicas {
            expected.services.push(InstanceService {
                name: name.clone(),
                status: "missing".into(),
                one_shot: labels[compose::ROLE].as_str() == Some("oneshot"),
                image: service["image"].as_str().map(str::to_owned),
                port: labels[compose::PORT]
                    .as_str()
                    .and_then(|port| port.parse().ok()),
                url: labels[compose::URL]
                    .as_str()
                    .map(|url| url.replace("$$", "$")),
                runtime: ServiceRuntime {
                    state: ContainerState::Missing,
                    replica,
                    ..Default::default()
                },
                ..Default::default()
            });
        }
    }
    Ok(Some(expected))
}
