use std::time::Instant;

use super::{
    command::Progress,
    config::Config,
    docker, journal, lifecycle, ownership, removal,
    runtime_db::{self, Kind},
    startup,
};
use crate::store::environments::{Instance, OperationState, StartupKind};

pub(super) fn unclaimed_startup(config: &Config, name: &str) -> Result<bool, String> {
    if journal::recorded(config, name)?.is_some()
        || runtime_db::load(config, name, Kind::Ownership)?.is_some()
        || runtime_db::load(config, name, Kind::Launch)?.is_some()
    {
        return Ok(false);
    }
    let Some(record) = startup::read(config, name)? else {
        return Ok(false);
    };
    let record = record.observe(config)?;
    Ok(record.owner_pid == 0
        && record.kind == StartupKind::Cold
        && record.operation.state == OperationState::Failed)
}

pub(super) fn recorded_workspace(config: &Config, name: &str) -> Result<Option<String>, String> {
    let Some(instance) = journal::recorded(config, name)? else {
        return Ok(None);
    };
    if !instance.workspace_only {
        ownership::verify(config, &instance)?;
    }
    removal::validate_workspace(config, name)?;
    Ok(Some(instance.workspace))
}

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
