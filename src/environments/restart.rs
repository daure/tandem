use std::{
    collections::{BTreeMap, BTreeSet},
    thread,
    time::{Duration, Instant},
};

use super::{
    command::{Progress, docker, remaining, run},
    config::Config,
    docker as runtime, gateway, lifecycle, templates,
};
use crate::store::environments::{Route, validate_instance_name};

pub(super) fn restart(
    config: &Config,
    name: &str,
    service: Option<&str>,
    timeout: u64,
    progress: Progress,
) -> Result<(), String> {
    validate_instance_name(name)?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let _lock = gateway::lock(config, &format!("instance-{name}"))?;
    let (instance, _) = lifecycle::managed_instance(config, name, deadline)?;
    let targets: Vec<_> = instance
        .services
        .iter()
        .filter(|target| !target.one_shot && service.is_none_or(|name| target.name == name))
        .collect();
    if targets.is_empty() {
        return Err(match service {
            Some(service) => format!("restartable service {service} not found in instance {name}"),
            None => "instance has no restartable services".into(),
        });
    }
    if targets.iter().any(|target| target.status == "paused") {
        return Err("unpause the selected containers before restarting".into());
    }
    let mut routes = BTreeMap::new();
    let _template_lock = if targets.iter().any(|target| target.url.is_some()) {
        let lock = gateway::shared_lock(config, &format!("template-{}", instance.template))?;
        let template = templates::get(config, &instance.template)?;
        if template.directory != instance.template_directory {
            return Err("instance belongs to a different template directory".into());
        }
        for target in targets.iter().filter(|target| target.url.is_some()) {
            let route =
                template.manifest.routes.get(&target.name).ok_or_else(|| {
                    format!("missing readiness configuration for {}", target.name)
                })?;
            routes.insert(target.name.clone(), route.clone());
        }
        Some(lock)
    } else {
        None
    };
    progress(format!(
        "Restarting {} existing container(s); preserving data and configuration",
        targets.len()
    ));
    let mut command = docker();
    command
        .arg("restart")
        .args(targets.iter().map(|target| &target.container_id));
    run(command, remaining(deadline)?, Some(progress.clone()))?;
    let ids = targets
        .iter()
        .map(|target| target.container_id.clone())
        .collect();
    wait_ready(config, name, &ids, &routes, deadline, progress)
}

pub(super) fn wait_ready(
    config: &Config,
    name: &str,
    ids: &BTreeSet<String>,
    routes: &BTreeMap<String, Route>,
    deadline: Instant,
    progress: Progress,
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?;
    let mut last_reason = "Waiting for restarted containers".to_owned();
    progress(last_reason.clone());
    loop {
        remaining(deadline).map_err(|_| {
            format!(
                "restart readiness timed out: {last_reason}; resources preserved for inspection"
            )
        })?;
        let mut instance = runtime::inspect_until(config, deadline)?
            .into_iter()
            .find(|instance| instance.name == name)
            .ok_or("instance disappeared during restart readiness")?;
        instance
            .services
            .retain(|service| ids.contains(&service.container_id));
        if instance.services.len() != ids.len() {
            return Err(
                "restarted containers disappeared or were replaced during readiness".into(),
            );
        }
        if let Some(service) = instance.services.iter().find(|service| {
            service.status.starts_with("down")
                || matches!(service.status.as_str(), "paused" | "removing")
        }) {
            return Err(format!(
                "{}: {}; inspect its container logs",
                service.name, service.status
            ));
        }
        let waiting = if let Some(service) = instance
            .services
            .iter()
            .find(|service| !matches!(service.status.as_str(), "up" | "healthy"))
        {
            Some(format!("Waiting for {}: {}", service.name, service.status))
        } else {
            lifecycle::readiness(&instance, routes, &client, deadline)?
        };
        match waiting {
            None => {
                progress(
                    "Restart ready: service health and configured gateway checks passed".into(),
                );
                return Ok(());
            }
            Some(reason) if reason != last_reason => {
                progress(reason.clone());
                last_reason = reason;
            }
            Some(_) => {}
        }
        thread::sleep(
            Duration::from_secs(1).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}
