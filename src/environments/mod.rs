mod command;
mod compose;
pub(crate) mod config;
mod containers;
mod creation;
mod docker;
mod gateway;
mod journal;
mod lifecycle;
mod ownership;
mod removal;
mod repositories;
mod resources;
mod stats;
mod templates;

use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use crate::store::environments::{
    EnvironmentSnapshot, Instance, InstanceService, Instructions, Operation, OperationState,
    StartupTiming, Template, validate_instance_name, validate_name,
};
use command::Progress;
use config::Config;
pub(crate) use creation::Startup;

pub(crate) struct Environments {
    pub config: Config,
    snapshot: Mutex<EnvironmentSnapshot>,
    inventory_errors: Mutex<[Option<String>; 2]>,
    operations: Mutex<BTreeMap<String, Job>>,
    next_id: AtomicU64,
    instance_revision: AtomicU64,
    resources: Mutex<resources::ResourceCache>,
}

struct Job {
    operation: Operation,
    started: Instant,
    pending_services: Vec<InstanceService>,
    started_at: u64,
    timeout_seconds: u64,
}

impl Job {
    fn running(&self) -> bool {
        self.operation.state == OperationState::Running
            && self.started.elapsed().as_secs() < self.timeout_seconds
    }
}

impl Environments {
    pub fn new(config: Config) -> Self {
        Self {
            snapshot: Mutex::new(EnvironmentSnapshot {
                templates_root: config.templates.display().to_string(),
                home_directory: dirs::home_dir().map(|path| path.display().to_string()),
                gateway_origin: config.origin(),
                loading: true,
                ..Default::default()
            }),
            config,
            inventory_errors: Mutex::new([None, None]),
            operations: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            instance_revision: AtomicU64::new(0),
            resources: Mutex::new(resources::ResourceCache::default()),
        }
    }

    pub fn snapshot(&self) -> EnvironmentSnapshot {
        let stored = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut snapshot = stored.clone();
        let jobs = self
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for job in jobs
            .values()
            .filter(|job| job.running() && job.operation.action == "create_instance")
        {
            snapshot.startup.insert(
                job.operation.name.clone(),
                StartupTiming {
                    elapsed_milliseconds: job
                        .started
                        .elapsed()
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX),
                    estimate_milliseconds: None,
                },
            );
        }
        for instance in &mut snapshot.instances {
            if instance
                .runtime
                .activity
                .as_ref()
                .is_some_and(|activity| activity.owner_pid == std::process::id())
            {
                instance.runtime.activity = None;
            }
            if let Some(job) = jobs
                .values()
                .find(|job| job.operation.targets(instance) && job.running())
            {
                instance.runtime.activity = Some(activity_from_job(job));
            } else if jobs.values().any(|job| {
                job.operation.targets(instance)
                    && job.operation.state == OperationState::Running
                    && !job.running()
            }) {
                instance.pending = false;
                instance.runtime.issue = Some("Operation timed out; refresh required".into());
            }
            if instance.pending || snapshot.startup.contains_key(&instance.name) {
                for service in &mut instance.services {
                    service.usage = None;
                }
            }
            project_instance(instance, snapshot.runtime_error.is_some(), journal::now());
        }
        snapshot.activities.retain_mut(|activity| {
            let Some(job) = jobs.get(&activity.id) else {
                return true;
            };
            if job.running() {
                return true;
            }
            if job.operation.action == activity.action
                && job.operation.name == activity.name
                && job.operation.state != OperationState::Succeeded
            {
                activity.finished = true;
                activity.error = activity_from_job(job).error;
                return true;
            }
            false
        });
        for job in jobs
            .values()
            .filter(|job| job.operation.state == OperationState::Running)
        {
            if !snapshot
                .activities
                .iter()
                .any(|activity| activity.id == job.operation.id)
            {
                snapshot.activities.push(activity_from_job(job));
            }
        }
        if !snapshot.instances.iter().any(|instance| {
            !instance.suppress_resources()
                && instance.services.iter().any(|service| {
                    service.consumes_resources() && service.runtime.resource_error.is_some()
                })
        }) {
            snapshot.resource_error = None;
        }
        snapshot
    }

    pub fn refresh_templates(&self) {
        let templates = templates::list(&self.config);
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let error = match templates {
            Ok(templates) => {
                snapshot.templates = templates;
                None
            }
            Err(error) => Some(error),
        };
        self.set_inventory_error(&mut snapshot, 0, error);
    }

    fn set_inventory_error(
        &self,
        snapshot: &mut EnvironmentSnapshot,
        index: usize,
        error: Option<String>,
    ) {
        let mut errors = self
            .inventory_errors
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        errors[index] = error;
        let messages: Vec<_> = errors.iter().flatten().cloned().collect();
        snapshot.error = (!messages.is_empty()).then(|| messages.join("\n"));
    }

    pub fn refresh_instances(&self) {
        let revision = self.instance_revision.load(Ordering::SeqCst);
        let instances = docker::inspect(&self.config).and_then(|mut instances| {
            let activities = journal::enrich(&self.config, &mut instances)?;
            self.snapshot
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .activities = activities;
            Ok(instances)
        });
        self.publish_instances(instances, revision);
    }

    fn publish_instances(&self, instances: Result<Vec<Instance>, String>, revision: u64) {
        let pending = self.pending_instances();
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // An inspection begun before readiness completed must not replace the ready snapshot.
        if revision != self.instance_revision.load(Ordering::SeqCst) {
            return;
        }
        let error = match instances {
            Ok(mut instances) => {
                merge_pending_instances(&mut instances, &pending);
                snapshot.instances = instances;
                snapshot.observed_at_unix_seconds = Some(journal::now());
                None
            }
            Err(error) => Some(format!(
                "Docker unavailable; instance list may be stale: {error}"
            )),
        };
        merge_pending_instances(&mut snapshot.instances, &pending);
        snapshot.runtime_error = error.clone();
        self.set_inventory_error(&mut snapshot, 1, error);
        snapshot.loading = false;
        self.resources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .apply(&mut snapshot.instances);
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
    pub fn update_template_manifest(
        &self,
        name: &str,
        manifest: crate::store::environments::Manifest,
    ) -> Result<Template, String> {
        templates::update_manifest(&self.config, name, manifest)
    }
    pub fn list_instances(&self) -> Result<crate::store::environments::RuntimeInventory, String> {
        let mut instances = docker::inspect(&self.config)?;
        let activities = journal::enrich(&self.config, &mut instances)?;
        self.resources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .apply(&mut instances);
        for instance in &mut instances {
            project_instance(instance, false, journal::now());
        }
        Ok(crate::store::environments::RuntimeInventory {
            instances,
            activities,
            observed_at_unix_seconds: journal::now(),
        })
    }
    pub fn workspace(&self, name: &str) -> Result<String, String> {
        validate_instance_name(name)?;
        let root = fs::canonicalize(&self.config.workspaces).map_err(|error| error.to_string())?;
        let workspace = self.config.workspaces.join(name);
        let metadata = fs::symlink_metadata(&workspace)
            .map_err(|error| format!("workspace {name}: {error}"))?;
        if !metadata.file_type().is_dir() {
            return Err(format!("workspace {name} must be a real directory"));
        }
        let workspace = fs::canonicalize(&workspace).map_err(|error| error.to_string())?;
        if workspace.parent() != Some(root.as_path()) {
            return Err(format!("workspace {name} escapes the workspace root"));
        }
        Ok(workspace.display().to_string())
    }
    pub fn instructions(&self) -> Result<Instructions, String> {
        Ok(Instructions {
            file: self.config.instructions.display().to_string(),
            markdown: config::read_text(&self.config.instructions)?,
            templates_root: self.config.templates.display().to_string(),
            workspaces_root: self.config.workspaces.display().to_string(),
            gateway_origin: self.config.origin(),
            manifest_schema: serde_json::to_value(schemars::schema_for!(
                crate::store::environments::Manifest
            ))
            .map_err(|error| error.to_string())?,
        })
    }

    pub fn begin(
        &self,
        action: &str,
        name: &str,
        template: Option<String>,
    ) -> Result<Operation, String> {
        self.begin_scoped(action, name, template, None)
    }

    pub fn begin_restart(&self, name: &str, service: Option<String>) -> Result<Operation, String> {
        if service.as_deref() == Some("") {
            return Err("service name must not be empty".into());
        }
        let action = if service.is_some() {
            "restart_service"
        } else {
            "restart_instance"
        };
        self.begin_scoped(action, name, None, service)
    }

    pub fn begin_service_state(
        &self,
        name: &str,
        service: String,
        running: bool,
    ) -> Result<Operation, String> {
        if service.is_empty() {
            return Err("service name must not be empty".into());
        }
        let action = if running {
            "start_service"
        } else {
            "stop_service"
        };
        self.begin_scoped(action, name, None, Some(service))
    }

    fn begin_scoped(
        &self,
        action: &str,
        name: &str,
        template: Option<String>,
        service: Option<String>,
    ) -> Result<Operation, String> {
        match action {
            "create_instance" | "stop_instance" | "delete_instance" | "restart_instance"
            | "restart_service" | "start_service" | "stop_service" => {
                crate::store::environments::validate_instance_name(name)?;
            }
            _ => validate_name(name)?,
        }
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
            "{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            self.next_id.fetch_add(1, Ordering::SeqCst)
        );
        let operation = Operation {
            id: id.clone(),
            action: action.into(),
            name: name.into(),
            template,
            service,
            state: OperationState::Running,
            progress: vec!["Queued".into()],
            elapsed_seconds: 0,
            elapsed_milliseconds: 0,
            error: None,
            instance: None,
        };
        operations.insert(
            id,
            Job {
                operation: operation.clone(),
                started: Instant::now(),
                pending_services: Vec::new(),
                started_at: journal::now(),
                timeout_seconds: 900,
            },
        );
        drop(operations);
        self.add_pending_instance(&operation);
        Ok(operation)
    }

    fn pending_instances(&self) -> Vec<Instance> {
        self.operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter(|job| job.running() && job.operation.action == "create_instance")
            .filter_map(|job| self.pending_instance(&job.operation, &job.pending_services))
            .collect()
    }

    fn add_pending_instance(&self, operation: &Operation) {
        let Some(instance) = self.pending_instance(operation, &[]) else {
            return;
        };
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !snapshot
            .instances
            .iter()
            .any(|current| current.name == instance.name)
        {
            snapshot.instances.push(instance);
        }
    }

    fn pending_instance(
        &self,
        operation: &Operation,
        services: &[InstanceService],
    ) -> Option<Instance> {
        if operation.action != "create_instance" {
            return None;
        }
        let template = operation.template.as_ref()?;
        Some(Instance {
            name: operation.name.clone(),
            template: template.clone(),
            template_directory: self.config.templates.join(template).display().to_string(),
            workspace: self
                .config
                .workspaces
                .join(&operation.name)
                .display()
                .to_string(),
            project: self.config.project(&operation.name),
            pending: true,
            services: services.to_vec(),
            ..Default::default()
        })
    }

    fn set_pending_services(&self, operation_id: &str, services: Vec<InstanceService>) {
        let operation = {
            let mut jobs = self
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(job) = jobs.get_mut(operation_id) else {
                return;
            };
            job.pending_services = services.clone();
            job.operation.clone()
        };
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(instance) = snapshot
            .instances
            .iter_mut()
            .find(|instance| instance.pending && instance.name == operation.name)
        {
            instance.services = services;
        }
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
                    operation.elapsed_milliseconds = job
                        .started
                        .elapsed()
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX);
                    if !job.running() {
                        operation.state = OperationState::Failed;
                        operation.error = Some("Operation timed out; refresh required".into());
                    }
                }
                if let Some(instance) = &mut operation.instance {
                    if operation.state != OperationState::Running {
                        instance.runtime.activity = None;
                    }
                    project_instance(instance, false, journal::now());
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

    pub fn execute(self: &Arc<Self>, operation: Operation, timeout: u64, startup: Startup) {
        let mut config = self.config.clone();
        config.operation_id = Some(operation.id.clone());
        if let Some(job) = self
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(&operation.id)
        {
            job.timeout_seconds = timeout;
        }
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
                    &config,
                    operation.template.as_deref().ok_or("template required")?,
                    &operation.name,
                    startup,
                    timeout,
                    progress,
                    |services| self.set_pending_services(&operation.id, services),
                )
                .map(Some),
                "stop_instance" => {
                    lifecycle::stop(&config, &operation.name, progress).map(|()| None)
                }
                "restart_instance" | "restart_service" | "start_service" | "stop_service" => {
                    containers::change_state(
                        &config,
                        &operation.name,
                        operation.service.as_deref(),
                        match operation.action.as_str() {
                            "start_service" => "start",
                            "stop_service" => "stop",
                            _ => "restart",
                        },
                        timeout,
                        progress,
                    )
                    .map(|()| None)
                }
                "delete_instance" => {
                    lifecycle::delete(&config, &operation.name, progress).map(|()| None)
                }
                "stop_template" => {
                    lifecycle::stop_template(&config, &operation.name, progress).map(|()| None)
                }
                "delete_template" => {
                    lifecycle::delete_template(&config, &operation.name, progress).map(|()| None)
                }
                "create_template" => self.create_template(&operation.name).map(|_| None),
                "remove_template" => {
                    removal::template(&config, &operation.name, timeout, progress).map(|()| None)
                }
                _ => Err("unknown operation".into()),
            }
        }))
        .unwrap_or_else(|_| {
            Err("operation worker failed; inspect runtime state before retrying".into())
        });
        self.finish_operation(&operation.id, result);
    }

    #[cfg(test)]
    pub(crate) fn complete_instance_for_tests(&self, id: &str, instance: Instance) {
        self.finish_operation(id, Ok(Some(instance)));
    }

    fn finish_operation(&self, id: &str, result: Result<Option<Instance>, String>) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        {
            let mut jobs = self
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(job) = jobs.get_mut(id) {
                job.operation.elapsed_seconds = job.started.elapsed().as_secs();
                job.operation.elapsed_milliseconds = job
                    .started
                    .elapsed()
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX);
                match result {
                    Ok(mut instance) => {
                        if let Some(ready) = &mut instance {
                            ready.pending = false;
                            ready.runtime.activity = None;
                            project_instance(ready, false, journal::now());
                        }
                        if let Some(ready) = &instance {
                            if let Some(current) = snapshot
                                .instances
                                .iter_mut()
                                .find(|current| current.name == ready.name)
                            {
                                *current = ready.clone();
                            } else {
                                snapshot.instances.push(ready.clone());
                            }
                            self.resources
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .apply(&mut snapshot.instances);
                            self.instance_revision.fetch_add(1, Ordering::SeqCst);
                        }
                        job.operation.state = OperationState::Succeeded;
                        job.operation.instance = instance;
                    }
                    Err(error) => {
                        job.operation.state = OperationState::Failed;
                        job.operation.error = Some(error);
                        if let Some(instance) = snapshot
                            .instances
                            .iter_mut()
                            .find(|instance| instance.name == job.operation.name)
                        {
                            instance.pending = false;
                            instance.runtime.activity = None;
                            instance.runtime.issue = job.operation.error.clone();
                        }
                    }
                }
            }
        }
    }
}

fn merge_pending_instances(instances: &mut Vec<Instance>, pending: &[Instance]) {
    for pending in pending {
        let Some(instance) = instances
            .iter_mut()
            .find(|instance| instance.name == pending.name)
        else {
            instances.push(pending.clone());
            continue;
        };
        for service in &pending.services {
            if !instance.services.iter().any(|current| {
                current.name == service.name
                    && current.runtime.replica.max(1) == service.runtime.replica.max(1)
            }) {
                instance.services.push(service.clone());
            }
        }
    }
}

fn activity_from_job(job: &Job) -> crate::store::environments::Activity {
    let operation = &job.operation;
    crate::store::environments::Activity {
        id: operation.id.clone(),
        name: operation.name.clone(),
        template: operation.template.clone().or_else(|| {
            matches!(
                operation.action.as_str(),
                "stop_template" | "delete_template" | "remove_template"
            )
            .then(|| operation.name.clone())
        }),
        service: operation.service.clone(),
        action: operation.action.clone(),
        owner_pid: std::process::id(),
        started_at: job.started_at,
        deadline: job
            .started_at
            .saturating_add(job.timeout_seconds)
            .saturating_add(1),
        error: operation.error.clone().or_else(|| {
            (operation.state == OperationState::Running && !job.running())
                .then(|| "Operation timed out; refresh required".into())
        }),
        finished: !job.running(),
    }
}

fn project_instance(instance: &mut Instance, stale: bool, now: u64) {
    instance.runtime.stale = stale;
    let suppressed = instance.suppress_resources();
    for service in &mut instance.services {
        service.runtime.stale = stale;
        service.runtime.resources_suppressed = suppressed;
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
                        || matches!(
                            activity.action.as_str(),
                            "create_instance"
                                | "stop_instance"
                                | "delete_instance"
                                | "stop_template"
                                | "delete_template"
                                | "remove_template"
                        ))
            })
            .cloned();
        if suppressed || !service.consumes_resources() {
            service.usage = None;
        }
        if service.state() == crate::store::environments::ContainerState::Paused
            && let Some(usage) = &mut service.usage
        {
            usage.cpu_basis_points = None;
        }
        service.runtime.resource_age_seconds = service
            .usage
            .map(|usage| now.saturating_sub(usage.sampled_at_unix_seconds));
        service.summary = service.status_summary();
    }
    instance.summary = instance.status_summary();
}

#[cfg(test)]
mod tests;
