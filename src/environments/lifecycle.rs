use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use super::{
    command::{Progress, docker, remaining, run},
    compose,
    config::Config,
    docker as runtime, gateway, templates,
};
use crate::store::environments::{Instance, Template, validate_name};

pub(crate) fn start(
    config: &Config,
    template_name: &str,
    name: &str,
    timeout: u64,
    progress: Progress,
) -> Result<Instance, String> {
    validate_name(name)?;
    validate_name(template_name)?;
    let _lock = gateway::lock(config, &format!("instance-{name}"))?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let template = templates::get(config, template_name)?;
    let existing_ids = runtime::project_ids(config, name, deadline)?;
    if !existing_ids.is_empty() {
        let existing = runtime::inspect_until(config, deadline)?
            .into_iter()
            .find(|instance| instance.name == name)
            .ok_or("project name is occupied by containers not managed by Tandem")?;
        if existing.template_directory != template.directory
            || existing.services.len() != existing_ids.len()
        {
            return Err("instance name belongs to another template or unmanaged containers".into());
        }
    }
    let workspace = config.workspaces.join(name);
    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    if !fs::canonicalize(&workspace)
        .map_err(|error| error.to_string())?
        .starts_with(&config.workspaces)
    {
        return Err("workspace escapes workspace root".into());
    }
    progress("Validating Compose and rendering instance routes".into());
    let rendered = compose::render(
        config,
        &template,
        name,
        remaining(deadline)?.min(Duration::from_secs(30)),
    )?;
    gateway::ensure(config, progress.clone(), remaining(deadline)?)?;
    progress(format!(
        "Starting {} from {}",
        config.project(name),
        template.directory
    ));
    let mut command = compose::command(config, Path::new(&template.directory), &rendered, name);
    command.args(["up", "--detach", "--remove-orphans"]);
    run(command, remaining(deadline)?, Some(progress.clone()))?;
    progress("Waiting for service health and gateway content assertions".into());
    wait_ready(config, &template, name, deadline, progress)
}

fn wait_ready(
    config: &Config,
    template: &Template,
    name: &str,
    deadline: Instant,
    progress: Progress,
) -> Result<Instance, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|error| error.to_string())?;
    let mut last_reason = String::new();
    loop {
        remaining(deadline).map_err(|_| {
            format!("readiness timed out: {last_reason}; resources preserved for inspection")
        })?;
        let instance = runtime::inspect_until(config, deadline)?
            .into_iter()
            .find(|instance| instance.name == name);
        let verdict = match &instance {
            Some(instance) => readiness(instance, template, &client, deadline),
            None => Ok(Some("Waiting for instance containers".into())),
        }?;
        match verdict {
            None => {
                progress(
                    if template.manifest.routes.is_empty() {
                        "Services running; no public routes declared"
                    } else {
                        "Ready: gateway content assertions passed"
                    }
                    .into(),
                );
                return instance.ok_or_else(|| "instance disappeared during readiness".into());
            }
            Some(reason) if reason != last_reason => {
                progress(reason.clone());
                last_reason = reason;
            }
            Some(_) => {}
        }
        thread::sleep(Duration::from_secs(1).min(remaining(deadline)?));
    }
}

fn readiness(
    instance: &Instance,
    template: &Template,
    client: &reqwest::blocking::Client,
    deadline: Instant,
) -> Result<Option<String>, String> {
    for service in &instance.services {
        if service.status.starts_with("down")
            || service.status == "unhealthy"
            || service.status == "paused"
        {
            return Err(format!(
                "{}: {}; inspect its container logs",
                service.name, service.status
            ));
        }
        if service.one_shot && service.status != "exited 0"
            || !service.one_shot && !["up", "healthy"].contains(&service.status.as_str())
        {
            return Ok(Some(format!(
                "Waiting for {}: {}",
                service.name, service.status
            )));
        }
    }
    for (name, route) in &template.manifest.routes {
        let Some(service) = instance
            .services
            .iter()
            .find(|service| &service.name == name)
        else {
            return Ok(Some(format!("Waiting for service {name}")));
        };
        let base = service
            .url
            .as_deref()
            .ok_or_else(|| format!("missing URL on {name}"))?;
        let url = format!("{base}{}", route.readiness_path);
        let response = client
            .get(&url)
            .timeout(remaining(deadline)?.min(Duration::from_secs(3)))
            .send();
        let Ok(response) = response else {
            return Ok(Some(format!("Waiting for gateway route {url}")));
        };
        if !response.status().is_success() {
            return Ok(Some(format!(
                "Waiting for {url}: HTTP {}",
                response.status()
            )));
        }
        let mut body = String::new();
        if response.take(1_048_576).read_to_string(&mut body).is_err()
            || !body.contains(&route.readiness_contains)
        {
            return Ok(Some(format!("Waiting for content assertion at {url}")));
        }
    }
    Ok(None)
}

pub(crate) fn stop(config: &Config, name: &str, progress: Progress) -> Result<(), String> {
    validate_name(name)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let _lock = gateway::lock(config, &format!("instance-{name}"))?;
    let instance = runtime::inspect_until(config, deadline)?
        .into_iter()
        .find(|instance| instance.name == name)
        .ok_or("instance not found")?;
    let ids = runtime::project_ids(config, name, deadline)?;
    let owned: BTreeSet<_> = instance
        .services
        .iter()
        .map(|service| service.container_id.as_str())
        .collect();
    // ps emits short IDs; compare against the independently inspected project membership.
    if ids.len() != owned.len()
        || ids
            .iter()
            .any(|id| !owned.iter().any(|full| full.starts_with(id)))
    {
        return Err("project contains unmanaged containers; refusing to stop it".into());
    }
    progress("Removing instance containers; keeping workspace and volumes".into());
    let mut command = docker();
    command.args(["rm", "--force"]).args(&ids);
    run(command, remaining(deadline)?, Some(progress.clone()))?;
    let mut command = docker();
    command.args([
        "network",
        "ls",
        "--quiet",
        "--filter",
        &format!("label=com.docker.compose.project={}", config.project(name)),
    ]);
    let networks = run(
        command,
        remaining(deadline)?.min(Duration::from_secs(15)),
        None,
    )?;
    if !networks.trim().is_empty() {
        let mut command = docker();
        command
            .args(["network", "rm"])
            .args(networks.split_whitespace());
        run(
            command,
            remaining(deadline)?.min(Duration::from_secs(30)),
            Some(progress),
        )?;
    }
    Ok(())
}
