use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{Config, Environments, Job, Startup, gateway, journal};
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
    pub kind: StartupKind,
    pub started_at: u64,
    pub timeout: u64,
    pub owner_pid: u32,
    pub workspace_ready: bool,
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

fn path(config: &Config, name: &str) -> Result<PathBuf, String> {
    validate_instance_name(name)?;
    Ok(journal::directory(config)?.join(format!("{}.startup.json", name.to_ascii_lowercase())))
}

pub(crate) fn read(config: &Config, name: &str) -> Result<Option<Record>, String> {
    let path = path(config, name)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err("startup record must be a regular file".into());
        }
        Ok(_) => {}
    }
    let record: Record = serde_json::from_str(&super::config::read_text(&path)?)
        .map_err(|error| error.to_string())?;
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
    Ok(Some(record))
}

fn write(config: &Config, record: &Record) -> Result<(), String> {
    let path = path(config, &record.operation.name)?;
    let bytes = serde_json::to_vec(record).map_err(|error| error.to_string())?;
    if bytes.len() > 262_144 {
        return Err("startup record exceeds 256 KiB".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("invalid startup path")?)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    file.persist(path).map_err(|error| error.to_string())?;
    journal::publish(config);
    Ok(())
}

pub(crate) fn records(config: &Config) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    for entry in fs::read_dir(journal::directory(config)?).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".startup.json"))
        else {
            continue;
        };
        if let Some(record) = read(config, name)? {
            records.push(record.observe(config)?);
        }
    }
    Ok(records)
}

pub(super) fn forget(config: &Config, name: &str) -> Result<(), String> {
    match fs::remove_file(path(config, name)?) {
        Ok(()) => {
            journal::publish(config);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) struct Writer {
    config: Config,
    record: Mutex<Record>,
    error: Mutex<Option<String>>,
}

impl Writer {
    fn update(&self, update: impl FnOnce(&mut Record)) {
        let mut record = self
            .record
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        update(&mut record);
        if let Err(error) = write(&self.config, &record) {
            crate::diagnostics::record_error(
                "cannot persist startup",
                &std::io::Error::other(error.clone()),
            );
            *self.error.lock().unwrap_or_else(|error| error.into_inner()) = Some(error);
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

    pub fn finish(&self, mut operation: Operation) -> Result<(), String> {
        if let Some(error) = self
            .error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        {
            operation.state = OperationState::Failed;
            operation.error = Some(format!("Startup status could not be persisted: {error}"));
        }
        let mut record = self
            .record
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        record.operation = operation;
        write(&self.config, &record)
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
