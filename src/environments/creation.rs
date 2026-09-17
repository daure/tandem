use std::time::{Duration, Instant};

use super::{Environments, docker, gateway, lifecycle};
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
