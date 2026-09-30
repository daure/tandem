use super::{Environments, startup};
use crate::store::environments::{InstanceInspection, validate_instance_name};

impl Environments {
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
