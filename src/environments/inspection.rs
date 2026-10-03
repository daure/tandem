use super::{Environments, journal, startup};
use crate::store::environments::{Instance, InstanceInspection, validate_instance_name};

impl Environments {
    pub(crate) fn session_instance(&self, name: &str) -> Result<Instance, String> {
        validate_instance_name(name)?;
        if let Some(instance) = journal::workspace_instance(&self.config, name)? {
            return Ok(instance);
        }
        let inventory = self.list_instances()?;
        let mut instance = inventory
            .instances
            .into_iter()
            .find(|instance| instance.name == name)
            .ok_or("instance is unavailable; refresh and try again")?;
        instance.runtime.stale |= inventory.runtime_error.is_some();
        Ok(instance)
    }

    pub(crate) fn inspect_instance(&self, name: &str) -> Result<InstanceInspection, String> {
        validate_instance_name(name)?;
        let inventory = self.list_instances()?;
        let instance = inventory
            .instances
            .into_iter()
            .find(|instance| instance.name == name);
        let activities: Vec<_> = inventory
            .activities
            .into_iter()
            .filter(|activity| activity.targets_instance_name(name))
            .collect();
        let startup = startup::read(&self.config, name)?
            .map(|record| record.observe(&self.config).map(|record| record.operation))
            .transpose()?;
        if instance.is_none() && activities.is_empty() && startup.is_none() {
            return Err(match inventory.runtime_error {
                Some(error) => format!("cannot determine whether instance {name} exists: {error}"),
                None => format!("instance {name} not found"),
            });
        }
        Ok(InstanceInspection {
            name: name.into(),
            instance,
            activities,
            startup,
            observed_at_unix_seconds: inventory.observed_at_unix_seconds,
            runtime_error: inventory.runtime_error,
        })
    }
}
