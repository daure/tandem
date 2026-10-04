use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use super::{
    Startup, cleanup,
    command::{Progress, docker, remaining, run},
    compose,
    config::Config,
    docker as runtime, gateway, journal, ownership, repositories, runtime_db, template_files,
    templates, workspace_agents,
};
use crate::store::environments::{
    Instance, Route, Template, validate_instance_name, validate_name,
};

pub(crate) fn start(
    config: &Config,
    template_name: &str,
    name: &str,
    startup: Startup,
    timeout: u64,
    progress: Progress,
    mut pending_services: impl FnMut(Vec<crate::store::environments::InstanceService>),
) -> Result<Instance, String> {
    validate_instance_name(name)?;
    validate_name(template_name)?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let _lock = match startup.instance_lock {
        Some(lock) => lock,
        None => gateway::lock(config, &format!("instance-{name}"))?,
    };
    let activity = journal::ActivityGuard::begin(config, name, "create_instance", None, timeout)?;
    let result = (|| {
        journal::activity_template(config, name, template_name)?;
        let _template_lock = gateway::shared_lock(config, &format!("template-{template_name}"))?;
        let template = templates::get(config, template_name)?;
        pending_services(compose::preview_services(config, &template, name));
        let existing_ids = if template.workspace_only() {
            Vec::new()
        } else {
            runtime::project_ids(config, name, deadline)?
        };
        if !startup.start_instance && !existing_ids.is_empty() {
            return Err(
                "preparation without startup requires an instance without containers".into(),
            );
        }
        if !existing_ids.is_empty() {
            let existing = runtime::inspect_until(config, deadline)?
                .into_iter()
                .find(|instance| instance.name == name)
                .ok_or("project name is occupied by containers not managed by Tandem")?;
            if existing.template_directory != template.directory
                || existing.services.len() != existing_ids.len()
            {
                return Err(
                    "instance name belongs to another template or unmanaged containers".into(),
                );
            }
        }
        let branch = startup.branch_instances.then_some(name);
        if existing_ids.is_empty()
            && journal::recorded(config, name)?.is_none()
            && let Some(before_creation) = startup.before_creation
        {
            super::removal::validate_workspace(config, name)?;
            let workspace = config.workspaces.join(name);
            fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
            super::removal::validate_workspace(config, name)?;
            before_creation(
                workspace.to_str().ok_or("invalid workspace path")?,
                deadline,
            )?;
        }
        journal::prepare(config, &template, name, startup.description.as_deref())?;
        let description = journal::recorded(config, name)?
            .ok_or("instance record missing")?
            .description;
        ownership::record(config, template_name, Path::new(&template.directory), name)?;
        let workspace = config.workspaces.join(name);
        fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
        if !fs::canonicalize(&workspace)
            .map_err(|error| error.to_string())?
            .starts_with(&config.workspaces)
        {
            return Err("workspace escapes workspace root".into());
        }
        super::removal::validate_workspace(config, name)?;
        template_files::prepare(&workspace, &template, deadline, progress.clone())?;
        super::instance_mcp::prepare_config(config, &workspace)?;
        let mut preparing = journal::recorded(config, name)?.ok_or("instance record missing")?;
        preparing.services = compose::preview_services(config, &template, name);
        progress("Preparing workspace AGENTS.md".into());
        let guidance = workspace_agents::prepare(
            config,
            &preparing,
            &template.manifest.repositories,
            !template.workspace_only() || !template.manifest.repositories.is_empty(),
        )?;
        if let Some(before_repositories) = startup.before_repositories {
            super::removal::validate_workspace(config, name)?;
            before_repositories(
                workspace.to_str().ok_or("invalid workspace path")?,
                deadline,
            )?;
        }
        super::removal::validate_workspace(config, name)?;
        repositories::prepare(
            &config.workspaces,
            &workspace,
            &template,
            branch,
            deadline,
            progress.clone(),
            |checkout| journal::checkout(config, name, checkout),
        )?;
        if template.workspace_only() {
            let instance =
                journal::workspace_instance(config, name)?.ok_or("workspace record missing")?;
            workspace_agents::finish(config, &instance, &template.manifest.repositories, guidance)?;
            journal::workspace_ready(config, name)?;
            if let Some(writer) = &startup.writer {
                writer.workspace_ready();
            }
            pending_services(Vec::new());
            progress("Workspace ready; no services specified".into());
            return journal::workspace_instance(config, name)?
                .ok_or("workspace record missing".into());
        }
        progress("Validating Compose and rendering instance routes".into());
        let rendered = compose::render(
            config,
            &template,
            name,
            &description,
            branch,
            remaining(deadline)?.min(Duration::from_secs(30)),
        )?;
        let compose::Rendered {
            path: rendered,
            services,
        } = rendered;
        progress("Finalizing workspace AGENTS.md".into());
        workspace_agents::finish(
            config,
            &Instance {
                name: name.into(),
                description: description.clone(),
                template: template.name.clone(),
                template_directory: template.directory.clone(),
                workspace: workspace.display().to_string(),
                project: config.project(name),
                services: services.clone(),
                ..Default::default()
            },
            &template.manifest.repositories,
            guidance,
        )?;
        if let Some(writer) = &startup.writer {
            writer.workspace_ready();
        }
        pending_services(services);
        if !startup.start_instance {
            let instance = journal::prepared_ready(config, name)?;
            progress("Workspace and Compose configuration ready; services not started".into());
            return Ok(instance);
        }
        gateway::ensure(config, progress.clone(), remaining(deadline)?)?;
        progress(format!(
            "Starting {} from {}",
            config.project(name),
            template.directory
        ));
        let mut command = compose::command(
            config,
            Path::new(&template.directory),
            &rendered,
            name,
            branch,
        );
        // A reused instance name may retain image tags built by another template.
        command.args(["up", "--detach", "--build", "--remove-orphans"]);
        run(command, remaining(deadline)?, Some(progress.clone()))?;
        progress("Waiting for service health and gateway content assertions".into());
        wait_ready(config, &template, name, deadline, progress)
    })();
    activity.finish(result)
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
        let mut observed = runtime::inspect_until(config, deadline)?;
        journal::enrich(config, &mut observed)?;
        let instance = observed.into_iter().find(|instance| instance.name == name);
        let verdict = match &instance {
            Some(instance) => readiness(instance, &template.manifest.routes, &client, deadline),
            None => Ok(Some("Waiting for instance containers".into())),
        }?;
        match verdict {
            None => {
                journal::readiness_passed(
                    config,
                    instance
                        .as_ref()
                        .ok_or("instance disappeared during readiness")?,
                )?;
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

pub(super) fn readiness(
    instance: &Instance,
    routes: &BTreeMap<String, Route>,
    client: &reqwest::blocking::Client,
    deadline: Instant,
) -> Result<Option<String>, String> {
    for service in &instance.services {
        if matches!(
            service.state(),
            crate::store::environments::ContainerState::Exited
                | crate::store::environments::ContainerState::Dead
                | crate::store::environments::ContainerState::Paused
        ) && !service.ready()
            || service.state() == crate::store::environments::ContainerState::Running
                && service.health_state() == crate::store::environments::HealthState::Unhealthy
        {
            return Err(format!(
                "{}: {}; inspect its container logs",
                service.name, service.status
            ));
        }
        if !service.ready() {
            return Ok(Some(format!(
                "Waiting for {}: {}",
                service.name, service.status
            )));
        }
    }
    for (name, route) in routes {
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
    stop_with_lock(config, name, progress, None)
}

pub(super) fn stop_with_lock(
    config: &Config,
    name: &str,
    progress: Progress,
    instance_lock: Option<gateway::Lock>,
) -> Result<(), String> {
    validate_instance_name(name)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let _lock = match instance_lock {
        Some(lock) => lock,
        None => gateway::lock(config, &format!("instance-{name}"))?,
    };
    let activity = journal::ActivityGuard::begin(config, name, "stop_instance", None, 60)?;
    let result = (|| {
        let (instance, _) = managed_instance(config, name, deadline)?;
        journal::activity_template(config, name, &instance.template)?;
        if instance.workspace_only {
            progress("No services specified; workspace preserved".into());
            return Ok(());
        }
        progress("Stopping instance containers; keeping workspace, volumes, and networks".into());
        journal::stop(
            config,
            &instance,
            &instance.services.iter().collect::<Vec<_>>(),
            deadline,
            progress,
            true,
        )
    })();
    activity.finish(result)
}

pub(super) fn delete(
    config: &Config,
    name: &str,
    progress: Progress,
    before_deletion: &super::BeforeDeletion<'_>,
) -> Result<(), String> {
    delete_with_provenance(config, name, None, progress, before_deletion)
}

pub(super) fn delete_for_provider(
    config: &Config,
    name: &str,
    operation: &str,
    progress: Progress,
    before_deletion: &super::BeforeDeletion<'_>,
) -> Result<(), String> {
    delete_with_provenance(config, name, Some(operation), progress, before_deletion)
}

fn delete_with_provenance(
    config: &Config,
    name: &str,
    operation: Option<&str>,
    progress: Progress,
    before_deletion: &super::BeforeDeletion<'_>,
) -> Result<(), String> {
    validate_instance_name(name)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let _lock = gateway::lock(config, &format!("instance-{name}"))?;
    if let Some(operation) = operation {
        match super::startup::read(config, name)? {
            Some(record) if record.operation.id == operation => {}
            Some(_) => {
                return Err(format!(
                    "instance {name} has a different startup identity; refusing provider deletion"
                ));
            }
            None => {
                let workspace_absent = match fs::symlink_metadata(config.workspaces.join(name)) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                    Ok(_) => false,
                    Err(error) => return Err(error.to_string()),
                };
                if workspace_absent
                    && journal::recorded(config, name)?.is_none()
                    && runtime::project_ids(config, name, deadline)?.is_empty()
                {
                    let workspace = config.workspaces.join(name);
                    before_deletion(
                        workspace.to_str().ok_or("invalid workspace path")?,
                        deadline,
                    )?;
                    return Ok(());
                }
                return Err(format!(
                    "instance {name} has no verifiable startup identity; refusing provider deletion"
                ));
            }
        }
    }
    let activity = journal::ActivityGuard::begin(config, name, "delete_instance", None, 60)?;
    let result = (|| {
        if cleanup::unclaimed_startup(config, name)? {
            // An unclaimed request proves no resource ownership; only its journals may be removed.
            progress("Removing failed startup request; preserving any unverified resources".into());
            return Ok(());
        }
        let workspace = cleanup::recorded_workspace(config, name)?;
        if let Some(workspace) = &workspace {
            progress("Closing associated OpenCode clients before inspecting containers".into());
            before_deletion(workspace, deadline)?;
        }
        let (instance, ids) = cleanup::target(config, name, deadline, &progress)?;
        journal::activity_template(config, name, &instance.template)?;
        if workspace.is_none() {
            progress("Closing associated OpenCode clients before deleting instance data".into());
            before_deletion(&instance.workspace, deadline)?;
        }
        if !instance.workspace_only {
            if !ids.is_empty() {
                progress("Removing instance containers".into());
                let mut command = docker();
                command.args(["rm", "--force", "--volumes"]).args(&ids);
                run(command, remaining(deadline)?, Some(progress.clone()))?;
            }
            remove_networks(config, name, deadline, progress.clone())?;
            remove_volumes(config, name, deadline, progress.clone())?;
        }
        remove_workspace(config, name, progress.clone())?;
        remove_rendered_compose(config, &instance.template, name, progress)?;
        Ok(())
    })();
    activity.finish(result)
}

pub(crate) fn stop_template(
    config: &Config,
    template_name: &str,
    progress: Progress,
) -> Result<(), String> {
    let instances = template_instances(config, template_name)?;
    if instances.is_empty() {
        return Err("template has no instances".into());
    }
    for instance in instances {
        if instance.services.iter().any(|service| {
            service.consumes_resources()
                || service.state() == crate::store::environments::ContainerState::Restarting
        }) {
            stop(config, &instance.name, progress.clone())?;
        }
    }
    Ok(())
}

pub(super) fn delete_template(
    config: &Config,
    template_name: &str,
    progress: Progress,
    before_deletion: &super::BeforeDeletion<'_>,
) -> Result<(), String> {
    let instances = template_instances(config, template_name)?;
    if instances.is_empty() {
        return Err("template has no instances".into());
    }
    for instance in instances {
        delete(config, &instance.name, progress.clone(), before_deletion)?;
    }
    Ok(())
}

fn template_instances(config: &Config, template_name: &str) -> Result<Vec<Instance>, String> {
    validate_name(template_name)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let _lock = gateway::lock(config, &format!("template-{template_name}"))?;
    let mut instances = journal::workspaces(config)?;
    let directory = config.templates.join(template_name);
    let names = ownership::instances(config, template_name, &directory)?;
    let only_workspaces = (instances
        .iter()
        .any(|instance| instance.template == template_name)
        || templates::get(config, template_name).is_ok_and(|template| template.workspace_only()))
        && names.iter().all(|name| {
            instances
                .iter()
                .any(|instance| &instance.name == name && instance.template == template_name)
        });
    if !only_workspaces {
        instances.extend(runtime::inspect_until(config, deadline)?);
    }
    Ok(instances
        .into_iter()
        .filter(|instance| instance.template == template_name)
        .collect())
}

pub(super) fn managed_instance(
    config: &Config,
    name: &str,
    deadline: Instant,
) -> Result<(Instance, Vec<String>), String> {
    if let Some(instance) = journal::workspace_instance(config, name)? {
        return Ok((instance, Vec::new()));
    }
    let instance = runtime::inspect_until(config, deadline)?
        .into_iter()
        .find(|instance| instance.name == name);
    let Some(instance) = instance else {
        if let Some(mut instance) =
            journal::recorded(config, name)?.filter(|instance| instance.runtime.prepared_only)
        {
            ownership::verify(config, &instance)?;
            if !runtime::project_ids(config, name, deadline)?.is_empty() {
                return Err("project contains unmanaged containers; refusing to change it".into());
            }
            journal::enrich(config, std::slice::from_mut(&mut instance))?;
            return Ok((instance, Vec::new()));
        }
        return Err("instance not found".into());
    };
    checked_project(config, instance, deadline)
}

pub(super) fn checked_project(
    config: &Config,
    instance: Instance,
    deadline: Instant,
) -> Result<(Instance, Vec<String>), String> {
    let ids = runtime::project_ids(config, &instance.name, deadline)?;
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
        return Err("project contains unmanaged containers; refusing to change it".into());
    }
    Ok((instance, ids))
}

fn remove_networks(
    config: &Config,
    name: &str,
    deadline: Instant,
    progress: Progress,
) -> Result<(), String> {
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
        progress("Removing instance networks".into());
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

fn remove_volumes(
    config: &Config,
    name: &str,
    deadline: Instant,
    progress: Progress,
) -> Result<(), String> {
    let mut command = docker();
    command.args([
        "volume",
        "ls",
        "--quiet",
        "--filter",
        &format!("label=com.docker.compose.project={}", config.project(name)),
    ]);
    let volumes = run(
        command,
        remaining(deadline)?.min(Duration::from_secs(15)),
        None,
    )?;
    if volumes.trim().is_empty() {
        return Ok(());
    }
    progress("Removing instance volumes".into());
    let mut command = docker();
    command
        .args(["volume", "rm"])
        .args(volumes.split_whitespace());
    run(
        command,
        remaining(deadline)?.min(Duration::from_secs(30)),
        Some(progress),
    )?;
    Ok(())
}

pub(super) fn remove_workspace(
    config: &Config,
    name: &str,
    progress: Progress,
) -> Result<(), String> {
    validate_instance_name(name)?;
    let workspace = config.workspaces.join(name);
    let metadata = match fs::symlink_metadata(&workspace) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.file_type().is_dir() {
        return Err("workspace must be a real directory, not a symlink".into());
    }
    let root = fs::canonicalize(&config.workspaces).map_err(|error| error.to_string())?;
    let workspace = fs::canonicalize(&workspace).map_err(|error| error.to_string())?;
    if workspace.parent() != Some(root.as_path()) {
        return Err("workspace escapes workspace root".into());
    }
    progress("Removing instance workspace".into());
    fs::remove_dir_all(workspace).map_err(|error| error.to_string())
}

pub(super) fn remove_rendered_compose(
    config: &Config,
    _template_name: &str,
    name: &str,
    progress: Progress,
) -> Result<(), String> {
    progress("Removing rendered Compose file".into());
    runtime_db::launch::remove(config, name)
}
