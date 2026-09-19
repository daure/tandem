use std::time::Instant;

use super::{command::Progress, config::Config, docker, journal, lifecycle, ownership, removal};
use crate::store::environments::Instance;

pub(super) fn target(
    config: &Config,
    name: &str,
    deadline: Instant,
    progress: &Progress,
) -> Result<(Instance, Vec<String>), String> {
    if let Some(instance) = journal::workspace_instance(config, name)? {
        return Ok((instance, Vec::new()));
    }
    let observed = docker::inspect_until(config, deadline)?
        .into_iter()
        .find(|instance| instance.name == name);
    let instance = match observed {
        Some(instance) => instance,
        None => {
            let mut instance = journal::recorded(config, name)?
                .ok_or("instance not found; no recorded ownership available for cleanup")?;
            ownership::verify(config, &instance)?;
            removal::validate_workspace(config, name)?;
            // Launch slots are not live containers; project membership is checked independently.
            instance.services.clear();
            progress(
                "Recovering cleanup from verified ownership; no managed containers found".into(),
            );
            instance
        }
    };
    lifecycle::checked_project(config, instance, deadline)
}
