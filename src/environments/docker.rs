use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use serde_json::Value;

use super::{
    command::{docker, remaining, run},
    compose,
    config::Config,
};
use crate::store::environments::{Instance, InstanceService};

pub(crate) fn inspect(config: &Config) -> Result<Vec<Instance>, String> {
    inspect_until(config, Instant::now() + Duration::from_secs(30))
}

pub(crate) fn inspect_until(config: &Config, deadline: Instant) -> Result<Vec<Instance>, String> {
    let mut command = docker();
    command.args([
        "ps",
        "--all",
        "--quiet",
        "--filter",
        &format!("label={}={}", compose::NAMESPACE, config.namespace),
        "--filter",
        &format!("label={}=instance", compose::KIND),
    ]);
    let ids = run(
        command,
        remaining(deadline)?.min(Duration::from_secs(15)),
        None,
    )?;
    if ids.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut command = docker();
    command.arg("inspect").args(ids.split_whitespace());
    let containers: Vec<Value> = serde_json::from_str(&run(
        command,
        remaining(deadline)?.min(Duration::from_secs(15)),
        None,
    )?)
    .map_err(|error| error.to_string())?;
    instances(config, &containers)
}

pub(crate) fn instances(config: &Config, containers: &[Value]) -> Result<Vec<Instance>, String> {
    let mut instances = BTreeMap::<String, Instance>::new();
    let prefix = format!("{}-", config.namespace);
    for container in containers {
        let labels = &container["Config"]["Labels"];
        let label = |name: &str| labels[name].as_str().unwrap_or("");
        let project = label("com.docker.compose.project");
        let Some(name) = project.strip_prefix(&prefix) else {
            continue;
        };
        if name == "gateway" || label(compose::NAMESPACE) != config.namespace {
            continue;
        }
        crate::store::environments::validate_name(name)?;
        let instance = instances.entry(name.into()).or_insert_with(|| Instance {
            name: name.into(),
            project: project.into(),
            template: label(compose::TEMPLATE).into(),
            template_directory: label(compose::DIRECTORY).into(),
            workspace: label(compose::WORKSPACE).into(),
            services: Vec::new(),
        });
        if instance.template_directory != label(compose::DIRECTORY)
            || instance.template != label(compose::TEMPLATE)
        {
            return Err(format!("conflicting template labels on project {project}"));
        }
        instance.services.push(InstanceService {
            name: label("com.docker.compose.service").into(),
            container_id: container["Id"].as_str().unwrap_or("").into(),
            status: service_status(&container["State"], label(compose::ROLE) == "oneshot"),
            one_shot: label(compose::ROLE) == "oneshot",
            url: labels[compose::URL].as_str().map(str::to_owned),
        });
    }
    for instance in instances.values_mut() {
        instance.services.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(instances.into_values().collect())
}

pub(crate) fn service_status(state: &Value, one_shot: bool) -> String {
    match state["Status"].as_str().unwrap_or("unknown") {
        "running" => match state["Health"]["Status"].as_str() {
            Some("healthy") => "healthy".into(),
            Some("unhealthy") => "unhealthy".into(),
            Some("starting") => "boot".into(),
            _ => "up".into(),
        },
        "exited" if one_shot && state["ExitCode"].as_i64() == Some(0) => "exited 0".into(),
        "exited" | "dead" => format!("down (exit {})", state["ExitCode"].as_i64().unwrap_or(-1)),
        "created" | "restarting" => "boot".into(),
        other => other.into(),
    }
}

pub(crate) fn project_ids(
    config: &Config,
    name: &str,
    deadline: Instant,
) -> Result<Vec<String>, String> {
    let mut command = docker();
    command.args([
        "ps",
        "--all",
        "--quiet",
        "--filter",
        &format!("label=com.docker.compose.project={}", config.project(name)),
    ]);
    Ok(run(
        command,
        remaining(deadline)?.min(Duration::from_secs(15)),
        None,
    )?
    .split_whitespace()
    .map(str::to_owned)
    .collect())
}
