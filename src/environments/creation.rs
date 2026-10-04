use std::time::{Duration, Instant};

use super::{Environments, docker, gateway, journal, lifecycle, templates};
use crate::store::environments::{Instance, validate_instance_name, validate_name};

type BeforeCreation = Box<dyn FnOnce(&str, Instant) -> Result<(), String> + Send>;

pub(crate) struct Startup {
    pub start_instance: bool,
    pub preserve_opencode_history: bool,
    pub origin_operation_id: Option<String>,
    pub branch_instances: bool,
    pub description: Option<String>,
    pub opencode: Option<crate::store::opencode::Launch>,
    pub opencode_result: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
    pub instance_lock: Option<gateway::Lock>,
    pub before_creation: Option<BeforeCreation>,
    pub before_repositories: Option<BeforeCreation>,
    pub writer: Option<std::sync::Arc<super::startup::Writer>>,
}

impl Default for Startup {
    fn default() -> Self {
        Self {
            start_instance: true,
            preserve_opencode_history: false,
            origin_operation_id: None,
            branch_instances: false,
            description: None,
            opencode: None,
            opencode_result: None,
            instance_lock: None,
            before_creation: None,
            before_repositories: None,
            writer: None,
        }
    }
}

impl Environments {
    pub(crate) fn admit_instance_start(
        &self,
        name: &str,
    ) -> Result<(Instance, gateway::Lock), String> {
        validate_instance_name(name)?;
        let lock = gateway::lock(&self.config, &format!("instance-{name}"))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let (instance, _) = lifecycle::managed_instance(&self.config, name, deadline)?;
        let template = templates::get(&self.config, &instance.template)?;
        if instance.template_directory != template.directory
            || instance.workspace != self.config.workspaces.join(name).display().to_string()
            || instance.workspace_only != template.workspace_only()
        {
            return Err(
                "instance belongs to a different template directory, workspace, or execution kind"
                    .into(),
            );
        }
        if instance
            .services
            .iter()
            .any(|service| service.state() == crate::store::environments::ContainerState::Paused)
        {
            return Err("unpause the instance containers before start".into());
        }
        Ok((instance, lock))
    }

    pub fn admit_new_instance(
        &self,
        name: &str,
        template: &str,
    ) -> Result<(Option<Instance>, gateway::Lock), String> {
        validate_instance_name(name)?;
        validate_name(template)?;
        let lock = gateway::lock(&self.config, &format!("instance-{name}"))?;
        if let Some(instance) = journal::workspace_instance(&self.config, name)? {
            if instance.template != template {
                return Err("instance name belongs to another template or workspace".into());
            }
            return Ok((instance.runtime.workspace_ready.then_some(instance), lock));
        }
        if templates::get(&self.config, template).is_ok_and(|template| template.workspace_only()) {
            if journal::recorded(&self.config, name)?.is_some() {
                return Err("instance name belongs to a container-backed instance".into());
            }
            return Ok((None, lock));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        if docker::project_ids(&self.config, name, deadline)?.is_empty() {
            if let Some(mut instance) = journal::recorded(&self.config, name)?.filter(|instance| {
                instance.runtime.prepared_only && instance.runtime.workspace_ready
            }) {
                super::ownership::verify(&self.config, &instance)?;
                if instance.template != template {
                    return Err("instance name belongs to another template".into());
                }
                journal::enrich(&self.config, std::slice::from_mut(&mut instance))?;
                return Ok((Some(instance), lock));
            }
            return Ok((None, lock));
        }
        let (instance, _) = lifecycle::managed_instance(&self.config, name, deadline)?;
        if instance.template != template
            || instance.workspace != self.config.workspaces.join(name).display().to_string()
        {
            return Err("instance name belongs to another template or workspace".into());
        }
        Ok((Some(instance), lock))
    }
}
