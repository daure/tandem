use std::{collections::BTreeMap, fs, time::Instant};

use serde::{Deserialize, Serialize};

use super::{
    command::{Progress, docker, remaining, run},
    config::Config,
    gateway,
    runtime_db::{self, Kind},
};
use crate::store::environments::{
    Activity, ContainerState, Instance, InstanceService, validate_instance_name,
};

mod launch;
mod pings;
mod workspaces;
pub(super) use pings::{record_ping, session_pings};
pub(super) use workspaces::{
    checkout, forget, prepare, prepared_ready, recorded, workspace_instance, workspace_ready,
    workspaces,
};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Record {
    expected: Option<Instance>,
    repositories: Vec<crate::store::environments::RepositoryCheckout>,
    stops: BTreeMap<String, StopReceipt>,
    activity: Option<Activity>,
    whole_stop: Vec<(String, Option<String>)>,
    readiness: BTreeMap<String, (Option<String>, u64)>,
    pings: BTreeMap<String, u64>,
}

#[derive(Deserialize, Serialize)]
struct StopReceipt {
    started_at: Option<String>,
    finished_at: Option<String>,
    exit_code: Option<i64>,
    requested_at: String,
    confirmed: bool,
}

pub(super) fn now() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

fn read(config: &Config, name: &str) -> Result<Record, String> {
    runtime_db::load(config, name, Kind::Journal)?
        .map(|text| {
            serde_json::from_str(&text).map_err(|error| format!("runtime journal: {error}"))
        })
        .transpose()
        .map(|record| record.unwrap_or_default())
}

fn write(config: &Config, name: &str, record: &Record) -> Result<(), String> {
    let text = encode(record)?;
    runtime_db::save(config, name, Kind::Journal, &text)
}

fn encode(record: &Record) -> Result<String, String> {
    let text = serde_json::to_string(record).map_err(|error| error.to_string())?;
    if text.len() > 262_144 {
        return Err("runtime journal exceeds 256 KiB; reduce the instance topology or retained error output".into());
    }
    Ok(text)
}

pub(super) fn validate_legacy(
    config: &Config,
    key: &str,
    text: &str,
) -> Result<Option<Instance>, String> {
    let record: Record = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if let Some(expected) = &record.expected {
        workspaces::validate(config, &expected.name, expected)?;
        if expected.name.to_ascii_lowercase() != key {
            return Err("legacy journal name mismatch".into());
        }
    }
    if let Some(activity) = &record.activity {
        validate_instance_name(&activity.name)?;
        if activity.name.to_ascii_lowercase() != key {
            return Err("legacy activity name mismatch".into());
        }
    }
    Ok(record.expected)
}

pub(super) fn record_launch(
    config: &Config,
    snapshot: &runtime_db::launch::Snapshot,
    services: Vec<InstanceService>,
    description: &str,
) -> Result<(), String> {
    let mut record = read(config, &snapshot.name)?;
    record.expected = Some(Instance {
        name: snapshot.name.clone(),
        description: description.into(),
        template: snapshot.template.clone(),
        template_directory: snapshot.directory.display().to_string(),
        workspace: config.workspaces.join(&snapshot.name).display().to_string(),
        project: config.project(&snapshot.name),
        services,
        ..Default::default()
    });
    if let Some(activity) = &mut record.activity {
        activity.template = Some(snapshot.template.clone());
    }
    record.whole_stop.clear();
    let journal = encode(&record)?;
    let launch = snapshot.encode(config)?;
    runtime_db::save_many(
        config,
        &snapshot.name,
        &[(Kind::Journal, &journal), (Kind::Launch, &launch)],
    )
}

pub(super) struct ActivityGuard<'a> {
    config: &'a Config,
    name: String,
    finished: bool,
}

impl<'a> ActivityGuard<'a> {
    pub fn begin(
        config: &'a Config,
        name: &str,
        action: &str,
        service: Option<&str>,
        timeout: u64,
    ) -> Result<Self, String> {
        let mut record = read(config, name)?;
        let started_at = now();
        record.activity = Some(Activity {
            id: config.operation_id.clone().unwrap_or_else(|| {
                format!(
                    "{}-{}",
                    std::process::id(),
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                )
            }),
            name: name.into(),
            template: record
                .expected
                .as_ref()
                .map(|instance| instance.template.clone()),
            service: service.map(str::to_owned),
            action: action.into(),
            owner_pid: std::process::id(),
            started_at,
            deadline: started_at.saturating_add(timeout).saturating_add(1),
            error: None,
            finished: false,
        });
        write(config, name, &record)?;
        Ok(Self {
            config,
            name: name.into(),
            finished: false,
        })
    }

    pub fn finish<T>(mut self, result: Result<T, String>) -> Result<T, String> {
        let mut record = read(self.config, &self.name)?;
        if let Some(activity) = &mut record.activity {
            activity.finished = true;
            activity.error = result.as_ref().err().cloned();
            if activity.action == "delete_instance" && result.is_ok() {
                forget(self.config, &self.name)?;
                self.finished = true;
                return result;
            }
        }
        write(self.config, &self.name, &record)?;
        self.finished = true;
        result
    }
}

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let result = (|| {
            let mut record = read(self.config, &self.name)?;
            if let Some(activity) = &mut record.activity {
                activity.finished = true;
                activity.error = Some("Operation interrupted; refresh required".into());
            }
            write(self.config, &self.name, &record)
        })();
        if let Err(error) = result {
            crate::diagnostics::record_error(
                "cannot finish runtime activity",
                &std::io::Error::other(error),
            );
        }
    }
}

#[cfg(test)]
pub(super) fn topology(
    config: &Config,
    template: &str,
    name: &str,
    description: &str,
    services: Vec<InstanceService>,
) -> Result<(), String> {
    let mut record = read(config, name)?;
    record.expected = Some(Instance {
        name: name.into(),
        description: description.into(),
        template: template.into(),
        template_directory: config.templates.join(template).display().to_string(),
        workspace: config.workspaces.join(name).display().to_string(),
        project: config.project(name),
        services,
        ..Default::default()
    });
    if let Some(activity) = &mut record.activity {
        activity.template = Some(template.into());
    }
    record.whole_stop.clear();
    write(config, name, &record)
}

pub(super) fn update_description(
    config: &Config,
    instance: &Instance,
    description: &str,
) -> Result<(), String> {
    let mut record = read(config, &instance.name)?;
    let expected = record.expected.get_or_insert_with(|| instance.clone());
    if expected.name != instance.name
        || expected.project != instance.project
        || expected.template != instance.template
        || expected.template_directory != instance.template_directory
        || expected.workspace != instance.workspace
        || expected.workspace_only != instance.workspace_only
    {
        return Err(format!(
            "runtime topology ownership mismatch for {}",
            instance.name
        ));
    }
    expected.description = description.into();
    write(config, &instance.name, &record)
}

pub(super) fn activity_template(config: &Config, name: &str, template: &str) -> Result<(), String> {
    let mut record = read(config, name)?;
    if let Some(activity) = &mut record.activity {
        activity.template = Some(template.into());
    }
    write(config, name, &record)
}

pub(super) fn readiness_passed(config: &Config, instance: &Instance) -> Result<(), String> {
    let mut record = read(config, &instance.name)?;
    for service in instance
        .services
        .iter()
        .filter(|service| service.url.is_some())
    {
        record.readiness.insert(
            service.container_id.clone(),
            (service.started_at.clone(), now()),
        );
    }
    write(config, &instance.name, &record)
}

pub(super) fn enrich(config: &Config, instances: &mut [Instance]) -> Result<Vec<Activity>, String> {
    let mut activities = Vec::new();
    let mut records = BTreeMap::new();
    for (name, text) in runtime_db::list(config, Kind::Journal)? {
        let mut record: Record = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if let Some(activity) = &mut record.activity {
            if activity.active()
                && (activity.deadline <= now()
                    || !gateway::is_locked(config, &format!("instance-{}", activity.name))?)
            {
                activity.finished = true;
                activity.error = Some("Operation interrupted; refresh required".into());
            }
            if activity.active() || activity.error.is_some() {
                activities.push(activity.clone());
            }
        }
        records.insert(name, record);
    }
    for instance in instances {
        instance.runtime.observed_at = Some(now());
        let mut record = records
            .remove(&instance.name.to_ascii_lowercase())
            .unwrap_or_default();
        if record.expected.is_none() {
            record.expected = launch::read(config, instance)?;
        }
        if let Some(expected) = &record.expected {
            if expected.project != instance.project
                || expected.template_directory != instance.template_directory
                || expected.workspace_only != instance.workspace_only
            {
                return Err(format!(
                    "runtime topology ownership mismatch for {}",
                    instance.name
                ));
            }
            instance.runtime.topology_known = true;
            instance.description.clone_from(&expected.description);
            instance.runtime.workspace_ready = expected.runtime.workspace_ready;
            instance.runtime.prepared_only = expected.runtime.prepared_only
                && instance
                    .services
                    .iter()
                    .all(|service| service.container_id.is_empty());
            for service in &mut instance.services {
                service.runtime.unexpected = !expected.services.iter().any(|slot| {
                    slot.name == service.name
                        && slot.runtime.replica.max(1) == service.runtime.replica.max(1)
                });
            }
            for expected in &expected.services {
                if instance.services.iter().any(|service| {
                    service.name == expected.name
                        && service.runtime.replica.max(1) == expected.runtime.replica.max(1)
                }) {
                    continue;
                }
                let mut missing = expected.clone();
                missing.status = "missing".into();
                missing.runtime.state = ContainerState::Missing;
                missing.runtime.prepared_only = instance.runtime.prepared_only;
                missing.runtime.waiting = record.activity.as_ref().is_some_and(|activity| {
                    activity.active() && activity.action == "create_instance"
                });
                instance.services.push(missing);
            }
        }
        instance.repositories = record.repositories.clone();
        instance.runtime.activity = record
            .activity
            .as_ref()
            .filter(|activity| activity.active())
            .cloned();
        instance.runtime.issue = record
            .activity
            .as_ref()
            .and_then(|activity| activity.error.clone());
        for service in &mut instance.services {
            service.runtime.prepared_only =
                instance.runtime.prepared_only && service.container_id.is_empty();
        }
        if (instance.workspace_only || instance.runtime.prepared_only)
            && !fs::symlink_metadata(&instance.workspace)
                .is_ok_and(|metadata| metadata.file_type().is_dir())
        {
            instance.runtime.workspace_ready = false;
            instance.runtime.issue = Some("workspace is missing or is not a real directory".into());
        }
        instance.runtime.whole_stop = !record.whole_stop.is_empty()
            && instance.services.iter().all(|service| {
                record
                    .whole_stop
                    .contains(&(service.container_id.clone(), service.started_at.clone()))
            });
        for service in &mut instance.services {
            if let Some(receipt) = record.stops.get(&service.container_id) {
                service.runtime.requested_stop = receipt.confirmed
                    && receipt.started_at == service.started_at
                    && receipt.finished_at == service.runtime.finished_at
                    && receipt.exit_code == service.exit_code()
                    && service.state() == ContainerState::Exited;
            }
            if let Some((started, at)) = record
                .readiness
                .get(&service.container_id)
                .filter(|(started, _)| *started == service.started_at)
            {
                let _ = started;
                service.runtime.readiness_checked_at = Some(*at);
            }
            service.runtime.activity = instance
                .runtime
                .activity
                .as_ref()
                .filter(|activity| {
                    activity
                        .service
                        .as_ref()
                        .is_none_or(|name| name == &service.name)
                        && (!service.one_shot
                            || activity.action == "create_instance"
                            || activity.action == "stop_instance"
                            || activity.action == "delete_instance")
                })
                .cloned();
        }
    }
    Ok(activities)
}

pub(super) fn stop(
    config: &Config,
    before: &Instance,
    targets: &[&InstanceService],
    deadline: Instant,
    progress: Progress,
    whole: bool,
) -> Result<(), String> {
    let mut record = read(config, &before.name)?;
    let requested_at = chrono::Utc::now().to_rfc3339();
    for target in targets.iter().filter(|target| {
        matches!(
            target.state(),
            ContainerState::Running | ContainerState::Paused | ContainerState::Restarting
        )
    }) {
        record.stops.insert(
            target.container_id.clone(),
            StopReceipt {
                started_at: target.started_at.clone(),
                finished_at: None,
                exit_code: None,
                requested_at: requested_at.clone(),
                confirmed: false,
            },
        );
    }
    write(config, &before.name, &record)?;
    let mut command = docker();
    command
        .arg("stop")
        .args(targets.iter().map(|target| &target.container_id));
    let stopped = run(command, remaining(deadline)?, Some(progress));
    let observed = super::docker::inspect_until(config, deadline)?;
    let after = observed
        .iter()
        .find(|instance| instance.name == before.name)
        .ok_or("stop outcome unverified: instance disappeared")?;
    let mut confirmed = true;
    for target in targets {
        let current = after.services.iter().find(|service| {
            service.container_id == target.container_id && service.started_at == target.started_at
        });
        if current.is_none_or(|service| {
            !matches!(
                service.state(),
                ContainerState::Exited | ContainerState::Created
            )
        }) {
            confirmed = false;
            continue;
        }
        if let (Some(current), Some(receipt)) =
            (current, record.stops.get_mut(&target.container_id))
        {
            let after_request = current
                .runtime
                .finished_at
                .as_deref()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .zip(chrono::DateTime::parse_from_rfc3339(&receipt.requested_at).ok())
                .is_some_and(|(finished, requested)| finished >= requested);
            if after_request && !receipt.confirmed {
                receipt.confirmed = true;
                receipt.exit_code = current.exit_code();
                receipt.finished_at = current.runtime.finished_at.clone();
            }
        }
    }
    if whole && confirmed && stopped.is_ok() {
        record.whole_stop = after
            .services
            .iter()
            .map(|service| (service.container_id.clone(), service.started_at.clone()))
            .collect();
    }
    write(config, &before.name, &record)?;
    stopped?;
    if !confirmed {
        return Err(
            "stop outcome unverified: targeted containers changed or are still active".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/journal.rs"]
mod tests;
