use std::{
    collections::BTreeMap,
    process::Command,
    time::{Duration, Instant},
};

use serde_json::Value;

use super::{
    command::{Progress, docker, remaining, run},
    compose,
    config::Config,
};
use crate::store::environments::{
    ContainerState, HealthState, Instance, InstanceService, ServiceRuntime, validate_instance_name,
};

pub(crate) fn inspect(config: &Config) -> Result<Vec<Instance>, String> {
    inspect_until(config, Instant::now() + Duration::from_secs(30))
}

pub(crate) fn inspect_until(config: &Config, deadline: Instant) -> Result<Vec<Instance>, String> {
    inspect_with(config, deadline, &mut run)
}

pub(super) fn inspect_with(
    config: &Config,
    deadline: Instant,
    execute: &mut impl FnMut(Command, Duration, Option<Progress>) -> Result<String, String>,
) -> Result<Vec<Instance>, String> {
    for attempt in 0..3 {
        match inspect_once(config, deadline, execute) {
            Err(error) if attempt < 2 && container_disappeared(&error) => {
                std::thread::sleep(Duration::from_millis(40));
            }
            result => return result,
        }
    }
    unreachable!("bounded inspection retry returns from every iteration")
}

fn inspect_once(
    config: &Config,
    deadline: Instant,
    execute: &mut impl FnMut(Command, Duration, Option<Progress>) -> Result<String, String>,
) -> Result<Vec<Instance>, String> {
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
    let ids = execute(
        command,
        remaining(deadline)?.min(Duration::from_secs(15)),
        None,
    )?;
    if ids.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut command = docker();
    command.arg("inspect").args(ids.split_whitespace());
    let containers: Vec<Value> = serde_json::from_str(&execute(
        command,
        remaining(deadline)?.min(Duration::from_secs(15)),
        None,
    )?)
    .map_err(|error| error.to_string())?;
    instances(config, &containers)
}

fn container_disappeared(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("no such object") || error.contains("no such container")
}

pub(crate) fn instances(config: &Config, containers: &[Value]) -> Result<Vec<Instance>, String> {
    let mut instances = BTreeMap::<String, Instance>::new();
    let prefix = format!("{}-", config.namespace);
    for container in containers {
        let labels = &container["Config"]["Labels"];
        let label = |name: &str| labels[name].as_str().unwrap_or("");
        let project = label("com.docker.compose.project");
        let Some(project_name) = project.strip_prefix(&prefix) else {
            continue;
        };
        let name = label(compose::INSTANCE);
        let name = if name.is_empty() { project_name } else { name };
        if name.eq_ignore_ascii_case("gateway") || label(compose::NAMESPACE) != config.namespace {
            continue;
        }
        validate_instance_name(name)?;
        let instance = instances.entry(name.into()).or_insert_with(|| Instance {
            name: name.into(),
            description: label(compose::DESCRIPTION).into(),
            project: project.into(),
            template: label(compose::TEMPLATE).into(),
            template_directory: label(compose::DIRECTORY).into(),
            workspace: label(compose::WORKSPACE).into(),
            pending: false,
            services: Vec::new(),
            ..Default::default()
        });
        if instance.description != label(compose::DESCRIPTION)
            || instance.template_directory != label(compose::DIRECTORY)
            || instance.template != label(compose::TEMPLATE)
        {
            return Err(format!("conflicting template labels on project {project}"));
        }
        let port = labels[compose::PORT]
            .as_str()
            .or_else(|| {
                labels.as_object().and_then(|labels| {
                    labels.iter().find_map(|(key, value)| {
                        (key.starts_with("traefik.http.services.")
                            && key.ends_with(".loadbalancer.server.port"))
                        .then(|| value.as_str())
                        .flatten()
                    })
                })
            })
            .map(|port| {
                port.parse::<u16>()
                    .map_err(|_| format!("invalid port label on {project}"))
            })
            .transpose()?;
        instance.services.push(InstanceService {
            name: label("com.docker.compose.service").into(),
            container_id: container["Id"].as_str().unwrap_or("").into(),
            status: service_status(&container["State"], label(compose::ROLE) == "oneshot"),
            one_shot: label(compose::ROLE) == "oneshot",
            image: string_field(&container["Config"]["Image"]),
            health: string_field(&container["State"]["Health"]["Status"]),
            restart_policy: string_field(&container["HostConfig"]["RestartPolicy"]["Name"]),
            restart_count: container["RestartCount"].as_u64().unwrap_or_default(),
            created_at: string_field(&container["Created"]),
            started_at: string_field(&container["State"]["StartedAt"]),
            port,
            url: labels[compose::URL].as_str().map(str::to_owned),
            usage: None,
            memory_limit_bytes: container["HostConfig"]["Memory"]
                .as_u64()
                .filter(|limit| *limit > 0),
            volumes: container["Mounts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|mount| mount["Type"].as_str() == Some("volume"))
                .filter_map(|mount| string_field(&mount["Name"]))
                .collect(),
            runtime: ServiceRuntime {
                state: ContainerState::from_docker(
                    container["State"]["Status"].as_str().unwrap_or("unknown"),
                ),
                health: match container["State"]["Health"]["Status"].as_str() {
                    None => HealthState::Unconfigured,
                    Some("starting") => HealthState::Checking,
                    Some("healthy") => HealthState::Healthy,
                    Some("unhealthy") => HealthState::Unhealthy,
                    _ => HealthState::Unknown,
                },
                exit_code: container["State"]["ExitCode"].as_i64(),
                oom_killed: container["State"]["OOMKilled"].as_bool().unwrap_or(false),
                error: string_field(&container["State"]["Error"]),
                finished_at: string_field(&container["State"]["FinishedAt"]),
                replica: label("com.docker.compose.container-number")
                    .parse()
                    .unwrap_or(1),
                ..Default::default()
            },
            ..Default::default()
        });
    }
    for instance in instances.values_mut() {
        instance.services.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(instances.into_values().collect())
}

fn string_field(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|value| !value.is_empty() && !value.starts_with("0001-01-01"))
        .map(str::to_owned)
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
