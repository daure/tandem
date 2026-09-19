use std::time::{Duration, Instant};

use super::{Environments, docker, gateway, journal, lifecycle, templates};
use crate::store::environments::{Instance, validate_instance_name, validate_name};

#[derive(Default)]
pub(crate) struct Startup {
    pub branch_instances: bool,
    pub workspace_ready: Option<tokio::sync::oneshot::Sender<String>>,
    pub instance_lock: Option<gateway::Lock>,
}

impl Environments {
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
