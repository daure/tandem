use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{
    Config, Environments, Job, Startup, gateway, journal,
    runtime_db::{self, Kind},
};
use crate::store::environments::{
    Activity, Instance, InstanceService, Operation, OperationState, StartupKind,
    validate_instance_name,
};

mod process;
pub(crate) use process::{claim, launch};

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Record {
    pub operation: Operation,
    pub description: Option<String>,
    pub branch_instances: bool,
    #[serde(default = "Instance::default_start_instance")]
    pub start_instance: bool,
    pub kind: StartupKind,
    pub started_at: u64,
    pub timeout: u64,
    pub owner_pid: u32,
    pub workspace_ready: bool,
    #[serde(default)]
    pub opencode_requested: bool,
    #[serde(default)]
    pub opencode_result: Option<Result<(), String>>,
    pub services: Vec<InstanceService>,
}

impl Record {
    pub fn activity(&self) -> Activity {
        Activity {
            id: self.operation.id.clone(),
            name: self.operation.name.clone(),
            template: self.operation.template.clone(),
            service: None,
            action: "create_instance".into(),
            owner_pid: self.owner_pid,
            started_at: self.started_at,
            deadline: self
                .started_at
                .saturating_add(self.timeout)
                .saturating_add(1),
            error: self.operation.error.clone(),
            finished: self.operation.state != OperationState::Running,
        }
    }

    pub fn observe(mut self, config: &Config) -> Result<Self, String> {
        if self.operation.state == OperationState::Running {
            self.operation.elapsed_seconds = journal::now().saturating_sub(self.started_at);
            self.operation.elapsed_milliseconds =
                self.operation.elapsed_seconds.saturating_mul(1000);
            if self.operation.elapsed_seconds > self.timeout
                || !gateway::is_locked(config, &lease(&self.operation.id))?
            {
                // Completion is published before the lease is released.
                if let Some(latest) = read(config, &self.operation.name)?
                    && latest.operation.id == self.operation.id
                    && latest.operation.state != OperationState::Running
                {
                    return Ok(latest);
                }
                self.operation.state = OperationState::Failed;
                self.operation.error =
                    Some("Startup interrupted; inspect runtime state before retrying".into());
            }
        }
        Ok(self)
    }
}

fn lease(id: &str) -> String {
    format!("startup-{id}")
}

pub(crate) fn read(config: &Config, name: &str) -> Result<Option<Record>, String> {
    runtime_db::load(config, name, Kind::Startup)?
        .map(|text| decode(name, &text))
        .transpose()
}

pub(super) fn decode(name: &str, text: &str) -> Result<Record, String> {
    let record: Record = serde_json::from_str(text).map_err(|error| error.to_string())?;
    validate_instance_name(&record.operation.name)?;
    crate::store::environments::validate_name(
        record
            .operation
            .template
            .as_deref()
            .ok_or("startup template missing")?,
    )?;
    if !record.operation.name.eq_ignore_ascii_case(name)
        || record.operation.action != "create_instance"
        || record.operation.id.is_empty()
        || !record
            .operation
            .id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
        || !(5..=900).contains(&record.timeout)
    {
        return Err("invalid startup record identity".into());
    }
    Ok(record)
}

fn write(config: &Config, record: &Record) -> Result<(), String> {
    let text = encode(record)?;
    runtime_db::save(config, &record.operation.name, Kind::Startup, &text)
}

fn encode(record: &Record) -> Result<String, String> {
    let text = serde_json::to_string(record).map_err(|error| error.to_string())?;
    if text.len() > 262_144 {
        return Err("startup record exceeds 256 KiB".into());
    }
    Ok(text)
}

fn update(config: &Config, record: &Record) -> Result<(), String> {
    runtime_db::update_startup(
        config,
        &record.operation.name,
        &record.operation.id,
        &encode(record)?,
    )
}

pub(crate) fn records(config: &Config) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    for (name, text) in runtime_db::list(config, Kind::Startup)? {
        records.push(decode(&name, &text)?.observe(config)?);
    }
    Ok(records)
}

#[cfg(test)]
fn forget(config: &Config, name: &str) -> Result<(), String> {
    runtime_db::remove(config, name, &[Kind::Startup])
}

pub(crate) struct Writer {
    config: Config,
    record: Mutex<Record>,
}

impl Writer {
    fn update(&self, change: impl FnOnce(&mut Record)) {
        let mut record = self
            .record
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        change(&mut record);
        if let Err(error) = update(&self.config, &record) {
            crate::diagnostics::record_error(
                "cannot persist startup",
                &std::io::Error::other(error),
            );
        }
    }

    pub fn progress(&self, line: String) {
        self.update(|record| {
            record.operation.progress.push(line);
            if record.operation.progress.len() > 30 {
                record.operation.progress.remove(0);
            }
        });
    }

    pub fn services(&self, services: Vec<InstanceService>) {
        self.update(|record| record.services = services);
    }
    pub fn workspace_ready(&self) {
        self.update(|record| record.workspace_ready = true);
    }

    pub fn opencode_result(&self, result: Result<(), String>) {
        self.update(|record| record.opencode_result = Some(result));
    }

    pub fn finish(&self, operation: Operation) -> Result<(), String> {
        let mut record = self
            .record
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        record.operation = operation;
        update(&self.config, &record)
    }
}

impl Environments {
    pub(crate) fn refresh_startups(&self) -> Result<(), String> {
        let records = records(&self.config)?;
        let mut jobs = self
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        jobs.retain(|id, job| {
            !job.external || records.iter().any(|record| &record.operation.id == id)
        });
        for record in records {
            if self.config.operation_id.as_ref() == Some(&record.operation.id) {
                continue;
            }
            let elapsed = Duration::from_secs(journal::now().saturating_sub(record.started_at));
            jobs.insert(
                record.operation.id.clone(),
                Job {
                    operation: record.operation,
                    started: Instant::now()
                        .checked_sub(elapsed)
                        .unwrap_or_else(Instant::now),
                    pending_services: record.services,
                    started_at: record.started_at,
                    timeout_seconds: record.timeout,
                    startup_kind: Some(record.kind),
                    completion_generation: None,
                    external: true,
                    owner_pid: record.owner_pid,
                },
            );
        }
        Ok(())
    }

    pub(crate) fn adopt_startup(&self, record: &Record) {
        self.operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                record.operation.id.clone(),
                Job {
                    operation: record.operation.clone(),
                    started: Instant::now(),
                    pending_services: Vec::new(),
                    started_at: record.started_at,
                    timeout_seconds: record.timeout,
                    startup_kind: Some(record.kind),
                    completion_generation: None,
                    external: false,
                    owner_pid: std::process::id(),
                },
            );
    }
}

pub(super) fn enrich(
    config: &Config,
    instances: &mut Vec<Instance>,
    activities: &mut Vec<Activity>,
) -> Result<(), String> {
    for record in records(config)? {
        let operation = &record.operation;
        let active = operation.state == OperationState::Running;
        let instance = instances
            .iter_mut()
            .find(|instance| instance.name == operation.name);
        let instance = match instance {
            Some(instance) => instance,
            None if operation.state != OperationState::Succeeded => {
                let template = operation
                    .template
                    .as_ref()
                    .ok_or("startup template missing")?;
                instances.push(Instance {
                    name: operation.name.clone(),
                    template: template.clone(),
                    description: record.description.clone().unwrap_or_default(),
                    template_directory: config.templates.join(template).display().to_string(),
                    workspace: config
                        .workspaces
                        .join(&operation.name)
                        .display()
                        .to_string(),
                    project: config.project(&operation.name),
                    services: record.services.clone(),
                    ..Default::default()
                });
                instances.last_mut().ok_or("startup instance missing")?
            }
            None => continue,
        };
        instance.pending = active;
        if active {
            instance.runtime.activity = Some(record.activity());
        }
        if operation.error.is_some() {
            instance.runtime.issue = operation.error.clone();
        }
        if active || operation.error.is_some() {
            activities.retain(|activity| activity.id != operation.id);
            activities.push(record.activity());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/startup.rs"]
mod tests;
