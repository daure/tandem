use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use super::{
    command::{Progress, docker, remaining, run},
    config::Config,
    docker as runtime, gateway, journal, lifecycle, ownership, templates,
};
use crate::store::environments::{Instance, validate_instance_name};

struct InstancePlan {
    name: String,
    containers: Vec<String>,
    paused: Vec<String>,
    networks: Vec<String>,
    volumes: Vec<String>,
}

pub(super) fn template(
    config: &Config,
    name: &str,
    timeout: u64,
    progress: Progress,
    close_command: &super::close_command::CloseCommand,
) -> Result<(), String> {
    template_with(config, name, timeout, progress, close_command, run)
}

pub(super) fn template_with(
    config: &Config,
    name: &str,
    timeout: u64,
    progress: Progress,
    close_command: &super::close_command::CloseCommand,
    execute: impl FnMut(Command, Duration, Option<Progress>) -> Result<String, String>,
) -> Result<(), String> {
    crate::store::environments::validate_name(name)?;
    let _template_lock = gateway::lock(config, &format!("template-{name}"))?;
    let directory = templates::removal_directory(config, name)?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    if remove_workspace_template(config, name, &directory, deadline, &progress, close_command)? {
        return Ok(());
    }
    let mut remover = Remover {
        config,
        deadline,
        progress,
        close_command,
        execute,
    };
    let inventory = runtime::inspect_with(config, remover.deadline, &mut remover.execute)?;
    let owned: BTreeSet<_> = inventory
        .iter()
        .filter(|instance| {
            instance.template == name && Path::new(&instance.template_directory) == directory
        })
        .flat_map(|instance| &instance.services)
        .map(|service| &service.container_id)
        .collect();
    let references = remover.template_containers(&directory)?;
    if references.len() != owned.len()
        || references
            .iter()
            .any(|id| !owned.iter().any(|full| full.starts_with(id)))
    {
        return Err(
            "template has unmanaged, foreign-namespace, or changed containers; refusing deletion"
                .into(),
        );
    }
    let mut names = ownership::instances(config, name, &directory)?;
    for instance in &inventory {
        if instance.template == name && Path::new(&instance.template_directory) == directory {
            names.insert(instance.name.clone());
        }
    }
    let mut locks = Vec::new();
    let mut plans = Vec::new();
    let mut projects = BTreeSet::new();
    for instance_name in names {
        validate_instance_name(&instance_name)?;
        if !projects.insert(config.project(&instance_name)) {
            return Err("instance names collide on a Docker project".into());
        }
        locks.push(gateway::lock(config, &format!("instance-{instance_name}"))?);
        let instance = inventory
            .iter()
            .find(|instance| instance.name == instance_name);
        if instance.is_some_and(|instance| {
            instance.template != name || Path::new(&instance.template_directory) != directory
        }) {
            return Err(format!(
                "instance {instance_name} belongs to another template"
            ));
        }
        validate_workspace(config, &instance_name)?;
        plans.push(remover.plan(&instance_name, instance)?);
    }
    for plan in &plans {
        ownership::record(config, name, &directory, &plan.name)?;
    }
    for plan in &plans {
        if !plan.paused.is_empty() {
            remover.command(
                std::iter::once("unpause").chain(plan.paused.iter().map(String::as_str)),
                true,
            )?;
        }
        if !plan.containers.is_empty() {
            (remover.progress)(format!("Stopping instance {}", plan.name));
            remover.command(
                ["stop", "--time", "10"]
                    .into_iter()
                    .chain(plan.containers.iter().map(String::as_str)),
                true,
            )?;
        }
    }
    for plan in &plans {
        remover.remove_instance(plan)?;
        journal::forget(config, &plan.name)?;
    }
    if !remover.template_containers(&directory)?.is_empty() {
        return Err("template still has containers; retry deletion".into());
    }
    remaining(remover.deadline)?;
    let directory = templates::removal_directory(config, name)?;
    (remover.progress)(format!("Deleting template {name} and its files"));
    fs::remove_dir_all(directory).map_err(|error| format!("delete template {name}: {error}"))
}

fn remove_workspace_template(
    config: &Config,
    name: &str,
    directory: &Path,
    deadline: Instant,
    progress: &Progress,
    close_command: &super::close_command::CloseCommand,
) -> Result<bool, String> {
    if directory.join("compose.yaml").exists() {
        return Ok(false);
    }
    let mut names = ownership::instances(config, name, directory)?;
    names.extend(
        journal::workspaces(config)?
            .into_iter()
            .filter(|instance| instance.template == name)
            .map(|instance| instance.name),
    );
    let mut locks = Vec::new();
    for instance_name in &names {
        remaining(deadline)?;
        let Some(instance) = journal::workspace_instance(config, instance_name)? else {
            return Ok(false);
        };
        if instance.template != name {
            return Err("instance belongs to another template".into());
        }
        locks.push(gateway::lock(config, &format!("instance-{instance_name}"))?);
        validate_workspace(config, instance_name)?;
    }
    for instance_name in &names {
        remaining(deadline)?;
        lifecycle::remove_workspace(config, instance_name, progress.clone(), close_command)?;
        ownership::forget(config, name, instance_name)?;
        journal::forget(config, instance_name)?;
    }
    remaining(deadline)?;
    fs::remove_dir_all(directory).map_err(|error| format!("delete template {name}: {error}"))?;
    Ok(true)
}

struct Remover<'a, F> {
    config: &'a Config,
    deadline: Instant,
    progress: Progress,
    close_command: &'a super::close_command::CloseCommand,
    execute: F,
}

impl<F: FnMut(Command, Duration, Option<Progress>) -> Result<String, String>> Remover<'_, F> {
    fn template_containers(&mut self, directory: &Path) -> Result<Vec<String>, String> {
        let filter = format!(
            "label={}={}",
            super::compose::DIRECTORY,
            directory.display()
        );
        Ok(self
            .command(["ps", "--all", "--quiet", "--filter", &filter], false)?
            .split_whitespace()
            .map(str::to_owned)
            .collect())
    }

    fn command(
        &mut self,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
        mutation: bool,
    ) -> Result<String, String> {
        let mut command = docker();
        command.args(args);
        let timeout = remaining(self.deadline)?;
        (self.execute)(
            command,
            if mutation {
                timeout
            } else {
                timeout.min(Duration::from_secs(15))
            },
            mutation.then(|| self.progress.clone()),
        )
    }

    fn project_resources(&mut self, name: &str, kind: &str) -> Result<Vec<String>, String> {
        let filter = format!(
            "label=com.docker.compose.project={}",
            self.config.project(name)
        );
        let output = if kind == "container" {
            self.command(["ps", "--all", "--quiet", "--filter", &filter], false)?
        } else {
            self.command([kind, "ls", "--quiet", "--filter", &filter], false)?
        };
        Ok(output.split_whitespace().map(str::to_owned).collect())
    }

    fn plan(&mut self, name: &str, instance: Option<&Instance>) -> Result<InstancePlan, String> {
        let containers = self.project_resources(name, "container")?;
        let owned: BTreeSet<_> = instance
            .into_iter()
            .flat_map(|instance| &instance.services)
            .map(|service| &service.container_id)
            .collect();
        if containers.len() != owned.len()
            || containers
                .iter()
                .any(|id| !owned.iter().any(|full| full.starts_with(id)))
        {
            return Err(format!(
                "project {name} contains unmanaged or changed containers; refusing deletion"
            ));
        }
        if instance.is_some_and(|instance| {
            instance.project != self.config.project(name)
                || Path::new(&instance.workspace) != self.config.workspaces.join(name)
        }) {
            return Err(format!(
                "inconsistent project or workspace ownership for {name}"
            ));
        }
        let paused = instance
            .into_iter()
            .flat_map(|instance| &instance.services)
            .filter(|service| service.status == "paused")
            .map(|service| service.container_id.clone())
            .collect();
        Ok(InstancePlan {
            name: name.into(),
            containers,
            paused,
            networks: self.project_resources(name, "network")?,
            volumes: self.project_resources(name, "volume")?,
        })
    }

    fn remove_instance(&mut self, plan: &InstancePlan) -> Result<(), String> {
        (self.progress)(format!("Deleting instance {} and its data", plan.name));
        if !plan.containers.is_empty() {
            self.command(
                ["rm", "--force", "--volumes"]
                    .into_iter()
                    .chain(plan.containers.iter().map(String::as_str)),
                true,
            )?;
        }
        for (kind, names) in [("network", &plan.networks), ("volume", &plan.volumes)] {
            if !names.is_empty() {
                self.command(
                    [kind, "rm"]
                        .into_iter()
                        .chain(names.iter().map(String::as_str)),
                    true,
                )?;
            }
        }
        for kind in ["container", "network", "volume"] {
            if !self.project_resources(&plan.name, kind)?.is_empty() {
                return Err(format!(
                    "instance {} still has {kind} resources; retry deletion",
                    plan.name
                ));
            }
        }
        lifecycle::remove_workspace(
            self.config,
            &plan.name,
            self.progress.clone(),
            self.close_command,
        )
    }
}

pub(super) fn validate_workspace(config: &Config, name: &str) -> Result<(), String> {
    let path = config.workspaces.join(name);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(format!("workspace {name} must be a real directory"));
        }
        Ok(_) => {}
    }
    let root = fs::canonicalize(&config.workspaces).map_err(|error| error.to_string())?;
    if root != config.workspaces
        || fs::canonicalize(&path)
            .map_err(|error| error.to_string())?
            .parent()
            != Some(root.as_path())
    {
        return Err(format!("workspace {name} escapes the workspace root"));
    }
    Ok(())
}
