use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::oneshot;

mod developer;
mod recreation;

use super::AppService;
use crate::{
    environments::{Startup, config::Config, rules::RuleStore, startup},
    store::{
        environments::OperationState,
        events::{Error, Record},
        rules::{self, Acceptance, Definition, DispatchStatus, Preview, Rule, Snapshot},
    },
};

pub(super) struct Integration {
    pub(super) store: RuleStore,
    snapshot: Mutex<Snapshot>,
    revision: AtomicU64,
    polling: AtomicBool,
    last_poll: Mutex<Instant>,
}

#[cfg(test)]
#[path = "tests/rules.rs"]
mod tests;

impl Integration {
    pub(super) fn new(config: &Config) -> Result<Self, Error> {
        Ok(Self {
            store: RuleStore::open(config)?,
            snapshot: Mutex::new(Snapshot::default()),
            revision: AtomicU64::new(0),
            polling: AtomicBool::new(false),
            last_poll: Mutex::new(Instant::now() - Duration::from_secs(1)),
        })
    }

    fn publish_observation(
        &self,
        revision: u64,
        observed: Result<Snapshot, Error>,
        cycle: Result<(), Error>,
    ) -> bool {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.revision.load(Ordering::Acquire) != revision {
            return false;
        }
        match observed {
            Ok(current) => *snapshot = current,
            Err(error) => snapshot.error = Some(error.to_string()),
        }
        if let Err(error) = cycle {
            snapshot.error = Some(error.to_string());
        }
        true
    }

    fn publish_saved(&self, rule: &Rule) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(current) = snapshot
            .rules
            .iter_mut()
            .find(|current| current.definition.name == rule.definition.name)
        {
            if current.revision > rule.revision {
                return;
            }
            *current = rule.clone();
        } else {
            snapshot.rules.push(rule.clone());
            snapshot
                .rules
                .sort_by(|left, right| left.definition.name.cmp(&right.definition.name));
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
    }
}

struct PollGuard(Arc<Integration>);
impl Drop for PollGuard {
    fn drop(&mut self) {
        self.0.polling.store(false, Ordering::Release);
    }
}

impl AppService {
    pub(crate) fn rule_snapshot(&self) -> Snapshot {
        self.rules
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn poll_rules(&self) {
        let mut last = self
            .rules
            .last_poll
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if last.elapsed() < Duration::from_millis(500)
            || self.rules.polling.swap(true, Ordering::AcqRel)
        {
            return;
        }
        *last = Instant::now();
        let service = self.clone();
        self.runtime.spawn_blocking(move || {
            let _guard = PollGuard(service.rules.clone());
            let revision = service.rules.revision.load(Ordering::Acquire);
            let result = service
                .rules
                .store
                .remember_workspaces(
                    &service.environment_snapshot(),
                    &service.opencode_snapshot(),
                )
                .and_then(|()| service.rule_cycle());
            let observed = service.rules.store.snapshot();
            service
                .rules
                .publish_observation(revision, observed, result);
        });
    }

    pub(crate) fn validate_rule_prompt(&self, prompt: &str) -> Result<(), String> {
        rules::validate_prompt(prompt)
    }

    pub(crate) fn save_rule(
        &self,
        definition: Definition,
        expected_revision: Option<i64>,
        destination: Option<String>,
        confirmed: bool,
    ) -> oneshot::Receiver<Result<Rule, String>> {
        let service = self.clone();
        let (sender, receiver) = oneshot::channel();
        self.runtime.spawn_blocking(move || {
            let result = (|| {
                if definition.enabled && !confirmed {
                    return Err("confirmation_required: enabling or editing an enabled rule authorizes automatic template execution and model prompts for future events".into());
                }
                rules::validate(&definition)?;
                let template = service.environments.get_template(&definition.template)?;
                if let Some(error) = template.error {
                    return Err(error);
                }
                let destination = destination
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| service.opencode.current_zellij.clone());
                if definition.enabled {
                    if !service.opencode_enabled() {
                        return Err("OpenCode integration is disabled".into());
                    }
                    if destination.is_empty()
                        || destination.len() > 200
                        || destination.chars().any(char::is_control)
                    {
                        return Err("an enabled rule requires a valid target Zellij session".into());
                    }
                    service.runtime.handle().block_on(
                        service.opencode.observer.validate_rule_destination(&destination),
                    )?;
                    #[cfg(not(test))]
                    crate::environments::provider_sidecar::ensure(&service.environments.config)?;
                }
                service
                    .rules
                    .store
                    .save(definition, expected_revision, destination)
                    .map_err(|error| error.to_string())
            })();
            if let Ok(rule) = &result {
                service.rules.publish_saved(rule);
            }
            let _ = sender.send(result);
        });
        receiver
    }

    pub(crate) async fn list_rules(&self) -> Result<Snapshot, String> {
        let store = self.rules.store.clone();
        self.runtime
            .spawn_blocking(move || store.snapshot().map_err(|error| error.to_string()))
            .await
            .map_err(|error| error.to_string())?
    }

    pub(crate) async fn preview_rule(
        &self,
        definition: Definition,
        sequence: i64,
    ) -> Result<Preview, String> {
        let store = self.rules.store.clone();
        self.runtime
            .spawn_blocking(move || {
                rules::validate(&definition)?;
                let record = store.event(sequence).map_err(|error| error.to_string())?;
                let input = rules::event_input(
                    &record.event,
                    &record.provider,
                    record.sequence,
                    &record.received_at,
                );
                let matched = rules::matches(&definition, &input)?;
                Ok(Preview {
                    matched,
                    resolved_prompt: matched
                        .then(|| rules::render_prompt(&definition.initial_prompt, &input))
                        .transpose()?,
                    template: definition.template,
                    model: definition.model,
                })
            })
            .await
            .map_err(|error| error.to_string())?
    }

    pub(crate) fn retained_event(&self, sequence: i64) -> oneshot::Receiver<Result<Record, Error>> {
        let store = self.rules.store.clone();
        let (sender, receiver) = oneshot::channel();
        self.runtime.spawn_blocking(move || {
            let _ = sender.send(store.event(sequence));
        });
        receiver
    }

    pub(crate) async fn retry_acceptance(
        &self,
        id: i64,
        confirmed: bool,
    ) -> Result<Acceptance, String> {
        if !confirmed {
            return Err("confirmation_required: retrying provisions the assigned instance".into());
        }
        let store = self.rules.store.clone();
        self.runtime
            .spawn_blocking(move || {
                let _lease = store
                    .lease()
                    .map_err(|error| error.to_string())?
                    .ok_or("rule worker is busy; try again")?;
                let mut acceptance = store
                    .snapshot()
                    .map_err(|error| error.to_string())?
                    .acceptances
                    .into_iter()
                    .find(|row| row.id == id)
                    .ok_or("acceptance not found")?;
                if acceptance.status != DispatchStatus::Failed
                    || acceptance.launch_started_at.is_some()
                    || acceptance.resolved_prompt.is_none()
                {
                    return Err("only confirmed pre-launch failures can be retried; uncertain launches require inspection and explicit replay".into());
                }
                acceptance.status = DispatchStatus::Queued;
                acceptance.error = None;
                store.update(&acceptance, false).map_err(|error| error.to_string())?;
                Ok(acceptance)
            })
            .await
            .map_err(|error| error.to_string())?
    }

    fn rule_cycle(&self) -> Result<(), Error> {
        let store = &self.rules.store;
        let Some(_lease) = store.lease()? else {
            return Ok(());
        };
        store.evaluate()?;
        let snapshot = store.snapshot()?;
        let active = snapshot
            .acceptances
            .iter()
            .filter(|row| {
                matches!(
                    row.status,
                    DispatchStatus::Provisioning | DispatchStatus::Launching
                )
            })
            .count();
        let mut available = 4usize.saturating_sub(active);
        // Oldest accepted work gets the first available dispatch slot.
        for mut acceptance in snapshot.acceptances.into_iter().rev() {
            if store.provider_deleting(acceptance.event_sequence)? {
                continue;
            }
            if acceptance.status == DispatchStatus::Queued {
                if available == 0 || !store.admit_dispatch(acceptance.id)? {
                    continue;
                }
                available -= 1;
            }
            if !matches!(
                acceptance.status,
                DispatchStatus::Queued | DispatchStatus::Provisioning | DispatchStatus::Launching
            ) {
                continue;
            }
            if let Err(error) = self.advance_dispatch(&mut acceptance) {
                acceptance.status = if acceptance.launch_started_at.is_some() {
                    DispatchStatus::Uncertain
                } else {
                    DispatchStatus::Failed
                };
                acceptance.error = Some(error);
                store.update(&acceptance, false)?;
            }
        }
        Ok(())
    }

    fn advance_dispatch(&self, acceptance: &mut Acceptance) -> Result<(), String> {
        let store = &self.rules.store;
        let config = &self.environments.config;
        match acceptance.status {
            DispatchStatus::Queued => {
                if !self.opencode_enabled() {
                    return Err("OpenCode integration is disabled".into());
                }
                self.runtime.handle().block_on(
                    self.opencode
                        .observer
                        .validate_rule_destination(&acceptance.rule.zellij_session),
                )?;
                if let Some(record) = startup::read(config, &acceptance.instance)?
                    && acceptance.operation_id.as_deref() != Some(&record.operation.id)
                {
                    return Err("assigned instance has a different startup operation; inspect before retrying".into());
                }
                let (existing, lock) = self.environments.admit_new_instance(
                    &acceptance.instance,
                    &acceptance.rule.definition.template,
                )?;
                if existing.is_some() && acceptance.operation_id.is_none() {
                    return Err("assigned instance name is already owned; dispatch requires a fresh instance".into());
                }
                let description = format!("{}: {}", acceptance.rule_name, acceptance.event_summary);
                let operation = self.environments.begin_instance(
                    &acceptance.instance,
                    acceptance.rule.definition.template.clone(),
                    Some(&description),
                )?;
                acceptance.operation_id = Some(operation.id.clone());
                acceptance.status = DispatchStatus::Provisioning;
                store
                    .update(acceptance, true)
                    .map_err(|error| error.to_string())?;
                self.schedule_operation(
                    operation,
                    600,
                    Startup {
                        start_instance: acceptance.rule.definition.start_instance,
                        instance_lock: Some(lock),
                        description: Some(description),
                        ..Default::default()
                    },
                );
            }
            DispatchStatus::Provisioning => {
                let record = startup::read(config, &acceptance.instance)?
                    .ok_or("startup admission was interrupted; inspect the assigned instance before retrying")?
                    .observe(config)?;
                if acceptance.operation_id.as_deref() != Some(&record.operation.id) {
                    return Err("assigned instance has a different startup operation; inspect before retrying".into());
                }
                match record.operation.state {
                    OperationState::Running => return Ok(()),
                    OperationState::Failed => {
                        return Err(record
                            .operation
                            .error
                            .unwrap_or_else(|| "instance preparation failed".into()));
                    }
                    OperationState::Succeeded => {}
                }
                if !self.opencode_enabled() {
                    return Err("OpenCode integration is disabled".into());
                }
                self.runtime.handle().block_on(
                    self.opencode
                        .observer
                        .validate_rule_destination(&acceptance.rule.zellij_session),
                )?;
                let workspace = self.environments.workspace(&acceptance.instance)?;
                acceptance.status = DispatchStatus::Launching;
                acceptance.launch_started_at = Some(chrono::Utc::now().to_rfc3339());
                store
                    .update(acceptance, false)
                    .map_err(|error| error.to_string())?;
                let pane = self.runtime.handle().block_on(
                    self.opencode.observer.new_rule_session(
                        &workspace,
                        &acceptance.instance,
                        &acceptance.rule.zellij_session,
                        &acceptance.rule.definition.model,
                        acceptance
                            .resolved_prompt
                            .as_deref()
                            .ok_or("resolved prompt unavailable")?,
                        record
                            .operation
                            .instance
                            .as_ref()
                            .map(|instance| instance.session_instructions()),
                    ),
                )?;
                acceptance.pane = Some(pane);
                store
                    .update(acceptance, false)
                    .map_err(|error| error.to_string())?;
            }
            DispatchStatus::Launching => {
                let pane = acceptance.pane.as_ref().ok_or(
                    "session launch was interrupted with an uncertain outcome; it will not be sent again",
                )?;
                let workspace = self.environments.workspace(&acceptance.instance)?;
                let observed = self.runtime.handle().block_on(
                    self.opencode
                        .observer
                        .observe(std::slice::from_ref(&workspace), Default::default()),
                )?;
                if let Some(session) = observed
                    .sessions
                    .iter()
                    .find(|session| session.directory == workspace && session.panes.contains(pane))
                {
                    acceptance.session_id = Some(session.id.clone());
                    acceptance.status = DispatchStatus::Launched;
                    store
                        .update(acceptance, false)
                        .map_err(|error| error.to_string())?;
                } else if acceptance
                    .launch_started_at
                    .as_ref()
                    .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                    .is_some_and(|time| {
                        (chrono::Utc::now() - time.with_timezone(&chrono::Utc)).num_seconds() > 60
                    })
                {
                    return Err("OpenCode session was not confirmed within 60 seconds; inspect the assigned pane and workspace before replaying".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
}
