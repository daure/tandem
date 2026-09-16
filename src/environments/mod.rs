mod command;
mod compose;
pub(crate) mod config;
mod docker;
mod gateway;
mod lifecycle;
mod templates;

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use crate::store::environments::{
    EnvironmentSnapshot, Instructions, Operation, OperationState, Template, validate_name,
};
use command::Progress;
use config::Config;

pub(crate) struct Environments {
    pub config: Config,
    snapshot: Mutex<EnvironmentSnapshot>,
    polling: AtomicBool,
    operations: Mutex<BTreeMap<String, Job>>,
    next_id: AtomicU64,
}

struct Job {
    operation: Operation,
    started: Instant,
}

impl Environments {
    pub fn new(config: Config) -> Self {
        Self {
            snapshot: Mutex::new(EnvironmentSnapshot {
                templates_root: config.templates.display().to_string(),
                gateway_origin: config.origin(),
                loading: true,
                ..Default::default()
            }),
            config,
            polling: AtomicBool::new(false),
            operations: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn snapshot(&self) -> EnvironmentSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    pub fn begin_refresh(&self) -> bool {
        !self.polling.swap(true, Ordering::SeqCst)
    }

    pub fn refresh(&self) {
        let templates = templates::list(&self.config);
        let instances = docker::inspect(&self.config);
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut errors = Vec::new();
        match templates {
            Ok(templates) => snapshot.templates = templates,
            Err(error) => errors.push(error),
        }
        match instances {
            Ok(instances) => snapshot.instances = instances,
            Err(error) => errors.push(format!(
                "Docker unavailable; instance list may be stale: {error}"
            )),
        }
        snapshot.error = (!errors.is_empty()).then(|| errors.join("\n"));
        snapshot.loading = false;
        self.polling.store(false, Ordering::SeqCst);
    }

    pub fn list_templates(&self) -> Result<Vec<Template>, String> {
        templates::list(&self.config)
    }
    pub fn get_template(&self, name: &str) -> Result<Template, String> {
        templates::get(&self.config, name)
    }
    pub fn create_template(&self, name: &str) -> Result<Template, String> {
        templates::create(&self.config, name)
    }
    pub fn list_instances(&self) -> Result<Vec<crate::store::environments::Instance>, String> {
        docker::inspect(&self.config)
    }
    pub fn instructions(&self) -> Result<Instructions, String> {
        Ok(Instructions {
            file: self.config.instructions.display().to_string(),
            markdown: config::read_text(&self.config.instructions)?,
            templates_root: self.config.templates.display().to_string(),
            workspaces_root: self.config.workspaces.display().to_string(),
            gateway_origin: self.config.origin(),
        })
    }

    pub fn begin(
        &self,
        action: &str,
        name: &str,
        template: Option<String>,
    ) -> Result<Operation, String> {
        validate_name(name)?;
        if let Some(template) = &template {
            validate_name(template)?;
        }
        let mut operations = self
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if operations
            .values()
            .any(|job| job.operation.name == name && job.operation.state == OperationState::Running)
        {
            return Err("an operation with this name is already running".into());
        }
        if operations.len() >= 100 {
            let finished = operations
                .iter()
                .filter(|(_, job)| job.operation.state != OperationState::Running)
                .min_by_key(|(_, job)| job.started)
                .map(|(id, _)| id.clone());
            match finished {
                Some(id) => {
                    operations.remove(&id);
                }
                None => return Err("too many active operations".into()),
            }
        }
        let id = format!(
            "{}-{}",
            std::process::id(),
            self.next_id.fetch_add(1, Ordering::SeqCst)
        );
        let operation = Operation {
            id: id.clone(),
            action: action.into(),
            name: name.into(),
            template,
            state: OperationState::Running,
            progress: vec!["Queued".into()],
            elapsed_seconds: 0,
            error: None,
            instance: None,
        };
        operations.insert(
            id,
            Job {
                operation: operation.clone(),
                started: Instant::now(),
            },
        );
        Ok(operation)
    }

    pub fn operations(&self) -> Vec<Operation> {
        let jobs = self
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut ordered: Vec<_> = jobs.values().collect();
        ordered.sort_by_key(|job| job.started);
        ordered
            .into_iter()
            .map(|job| {
                let mut operation = job.operation.clone();
                if operation.state == OperationState::Running {
                    operation.elapsed_seconds = job.started.elapsed().as_secs();
                }
                operation
            })
            .collect()
    }

    pub fn operation(&self, id: &str) -> Result<Operation, String> {
        self.operations()
            .into_iter()
            .find(|operation| operation.id == id)
            .ok_or_else(|| {
                "operation not found in this process; use list_instances after reconnecting".into()
            })
    }

    pub fn execute(self: &Arc<Self>, operation: Operation, timeout: u64) {
        let environment = Arc::clone(self);
        let id = operation.id.clone();
        let progress: Progress = Arc::new(move |line| {
            let mut jobs = environment
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(job) = jobs.get_mut(&id) {
                if job.operation.progress.len() >= 30 {
                    job.operation.progress.remove(0);
                }
                job.operation.progress.push(line);
            }
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match operation.action.as_str() {
                "create_instance" => lifecycle::start(
                    &self.config,
                    operation.template.as_deref().ok_or("template required")?,
                    &operation.name,
                    timeout,
                    progress,
                )
                .map(Some),
                "stop_instance" => {
                    lifecycle::stop(&self.config, &operation.name, progress).map(|()| None)
                }
                "create_template" => self.create_template(&operation.name).map(|_| None),
                _ => Err("unknown operation".into()),
            }
        }))
        .unwrap_or_else(|_| {
            Err("operation worker failed; inspect runtime state before retrying".into())
        });
        {
            let mut jobs = self
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(job) = jobs.get_mut(&operation.id) {
                job.operation.elapsed_seconds = job.started.elapsed().as_secs();
                match result {
                    Ok(instance) => {
                        job.operation.state = OperationState::Succeeded;
                        job.operation.instance = instance;
                    }
                    Err(error) => {
                        job.operation.state = OperationState::Failed;
                        job.operation.error = Some(error);
                    }
                }
            }
        }
        if self.begin_refresh() {
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests;
