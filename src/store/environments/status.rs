use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Instance, InstanceService};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContainerState {
    Created,
    Running,
    Paused,
    Restarting,
    Exited,
    Removing,
    Dead,
    Missing,
    #[default]
    Unknown,
}

impl ContainerState {
    pub fn from_docker(value: &str) -> Self {
        match value {
            "created" => Self::Created,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "restarting" => Self::Restarting,
            "exited" => Self::Exited,
            "removing" => Self::Removing,
            "dead" => Self::Dead,
            "missing" => Self::Missing,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HealthState {
    #[default]
    Unconfigured,
    Checking,
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Severity {
    #[default]
    Muted,
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    NotStarted,
    Waiting,
    Running,
    CheckingHealth,
    Healthy,
    Unhealthy,
    Restarting,
    Paused,
    Stopped,
    Removing,
    ContainerError,
    Missing,
    #[default]
    Unknown,
    Stale,
    Completed,
    WorkspaceReady,
    Failed,
    Interrupted,
    Degraded,
    Creating,
    Starting,
    Stopping,
    Deleting,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotStarted => "Not started",
            Self::Waiting => "Waiting",
            Self::Running => "Running",
            Self::CheckingHealth => "Checking health",
            Self::Healthy => "Healthy",
            Self::Unhealthy => "Unhealthy",
            Self::Restarting => "Restarting",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
            Self::Removing => "Removing",
            Self::ContainerError => "Container error",
            Self::Missing => "Missing",
            Self::Unknown => "Unknown",
            Self::Stale => "Stale",
            Self::Completed => "Completed",
            Self::WorkspaceReady => "Workspace ready",
            Self::Failed => "Failed",
            Self::Interrupted => "Interrupted",
            Self::Degraded => "Degraded",
            Self::Creating => "Creating",
            Self::Starting => "Starting",
            Self::Stopping => "Stopping",
            Self::Deleting => "Deleting",
        }
    }

    pub fn severity(self) -> Severity {
        match self {
            Self::NotStarted | Self::Waiting | Self::Stopped => Severity::Muted,
            Self::Running
            | Self::CheckingHealth
            | Self::Creating
            | Self::Starting
            | Self::Stopping
            | Self::Deleting
            | Self::Removing => Severity::Info,
            Self::Healthy | Self::Completed | Self::WorkspaceReady => Severity::Success,
            Self::Unhealthy | Self::ContainerError | Self::Failed => Severity::Error,
            _ => Severity::Warning,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct StatusSummary {
    pub status: Status,
    pub label: String,
    pub severity: Severity,
    pub detail: Option<String>,
    pub detail_severity: Severity,
    pub busy: bool,
    pub running: usize,
    pub expected: usize,
}

impl StatusSummary {
    pub fn new(status: Status) -> Self {
        Self {
            status,
            label: status.label().into(),
            severity: status.severity(),
            ..Self::default()
        }
    }

    pub fn detail(mut self, text: impl Into<String>, severity: Severity) -> Self {
        self.detail = Some(text.into());
        self.detail_severity = severity;
        self
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct Activity {
    pub id: String,
    pub name: String,
    pub template: Option<String>,
    pub service: Option<String>,
    pub action: String,
    pub owner_pid: u32,
    pub started_at: u64,
    pub deadline: u64,
    pub error: Option<String>,
    pub finished: bool,
}

impl Activity {
    pub fn status(&self) -> Status {
        match self.action.as_str() {
            "create_instance" => Status::Starting,
            "start_service" => Status::Starting,
            "restart_instance" | "restart_service" => Status::Restarting,
            "stop_instance" | "stop_service" | "stop_template" => Status::Stopping,
            "delete_instance" | "delete_template" | "remove_template" => Status::Deleting,
            _ => Status::Unknown,
        }
    }

    pub fn active(&self) -> bool {
        !self.finished && self.error.is_none()
    }

    pub fn targets_instance_name(&self, name: &str) -> bool {
        matches!(
            self.action.as_str(),
            "create_instance"
                | "stop_instance"
                | "delete_instance"
                | "restart_instance"
                | "restart_service"
                | "start_service"
                | "stop_service"
        ) && self.name == name
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct ServiceRuntime {
    pub state: ContainerState,
    pub health: HealthState,
    pub exit_code: Option<i64>,
    pub oom_killed: bool,
    pub error: Option<String>,
    pub finished_at: Option<String>,
    pub replica: u64,
    pub unexpected: bool,
    pub requested_stop: bool,
    pub stale: bool,
    pub waiting: bool,
    pub activity: Option<Activity>,
    pub resource_error: Option<String>,
    pub resource_age_seconds: Option<u64>,
    pub resources_suppressed: bool,
    pub readiness_checked_at: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct InstanceRuntime {
    pub topology_known: bool,
    pub workspace_ready: bool,
    pub stale: bool,
    pub observed_at: Option<u64>,
    pub whole_stop: bool,
    pub activity: Option<Activity>,
    pub issue: Option<String>,
}

impl InstanceService {
    // Legacy status is retained in the wire contract; raw observations take precedence.
    pub fn state(&self) -> ContainerState {
        if self.runtime.state != ContainerState::Unknown {
            return self.runtime.state;
        }
        match self.status.as_str() {
            "up" | "healthy" | "unhealthy" | "boot" => ContainerState::Running,
            value if value.starts_with("down (exit ") || value == "exited 0" => {
                ContainerState::Exited
            }
            value => ContainerState::from_docker(value),
        }
    }

    pub fn health_state(&self) -> HealthState {
        if self.runtime.state != ContainerState::Unknown {
            return self.runtime.health;
        }
        match self.status.as_str() {
            "healthy" => HealthState::Healthy,
            "unhealthy" => HealthState::Unhealthy,
            "boot" => HealthState::Checking,
            _ => HealthState::Unconfigured,
        }
    }

    pub fn exit_code(&self) -> Option<i64> {
        self.runtime.exit_code.or_else(|| {
            if self.runtime.state != ContainerState::Unknown {
                return None;
            }
            if self.status == "exited 0" {
                return Some(0);
            }
            self.status
                .strip_prefix("down (exit ")?
                .strip_suffix(')')?
                .parse()
                .ok()
        })
    }

    pub fn ready(&self) -> bool {
        if self.one_shot {
            return self.state() == ContainerState::Exited && self.exit_code() == Some(0);
        }
        self.state() == ContainerState::Running
            && matches!(
                self.health_state(),
                HealthState::Healthy | HealthState::Unconfigured
            )
    }

    pub fn can_start(&self) -> bool {
        !self.one_shot
            && !self.runtime.stale
            && !self.container_id.is_empty()
            && matches!(
                self.state(),
                ContainerState::Created | ContainerState::Exited
            )
            && self
                .runtime
                .activity
                .as_ref()
                .is_none_or(|activity| !activity.active())
    }

    pub fn can_stop(&self) -> bool {
        !self.one_shot
            && !self.runtime.stale
            && matches!(
                self.state(),
                ContainerState::Running | ContainerState::Restarting | ContainerState::Paused
            )
            && self
                .runtime
                .activity
                .as_ref()
                .is_none_or(|activity| !activity.active())
    }

    pub fn can_restart(&self) -> bool {
        self.can_start() || self.can_stop() && self.state() != ContainerState::Paused
    }

    pub fn status_summary(&self) -> StatusSummary {
        let status = match self.state() {
            ContainerState::Created if self.one_shot || self.runtime.waiting => Status::Waiting,
            ContainerState::Created => Status::NotStarted,
            ContainerState::Running => match self.health_state() {
                HealthState::Unconfigured => Status::Running,
                HealthState::Checking => Status::CheckingHealth,
                HealthState::Healthy => Status::Healthy,
                HealthState::Unhealthy => Status::Unhealthy,
                HealthState::Unknown => Status::Unknown,
            },
            ContainerState::Exited if self.one_shot => {
                if self.exit_code() == Some(0) {
                    Status::Completed
                } else if self.runtime.requested_stop && !self.runtime.oom_killed {
                    Status::Interrupted
                } else if self.exit_code().is_some() {
                    Status::Failed
                } else {
                    Status::Unknown
                }
            }
            ContainerState::Exited => Status::Stopped,
            ContainerState::Paused => Status::Paused,
            ContainerState::Restarting => Status::Restarting,
            ContainerState::Removing => Status::Removing,
            ContainerState::Dead => Status::ContainerError,
            ContainerState::Missing if self.runtime.waiting => Status::Waiting,
            ContainerState::Missing => Status::Missing,
            ContainerState::Unknown => Status::Unknown,
        };
        let mut result = StatusSummary::new(status);
        result.busy = matches!(status, Status::CheckingHealth | Status::Removing)
            || self.one_shot && self.state() == ContainerState::Running;
        if self.runtime.oom_killed {
            result = result.detail("OOM killed", Severity::Error);
        } else if let Some(error) = &self.runtime.error {
            result = result.detail(error, Severity::Error);
        } else if self.state() == ContainerState::Exited {
            if self.runtime.requested_stop {
                result = result.detail("requested stop", Severity::Muted);
            } else if let Some(code) = self.exit_code().filter(|code| *code != 0) {
                result = result.detail(
                    format!("exit {code}"),
                    if self.one_shot {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                );
            }
        } else if status == Status::Restarting {
            result = result.detail(
                format!("{} restarts", self.restart_count),
                Severity::Warning,
            );
        }
        if self.runtime.stale {
            result.status = Status::Stale;
            result.label = "Stale".into();
            result.severity = Severity::Warning;
            result.busy = false;
        }
        if let Some(activity) = self.runtime.activity.as_ref().filter(|activity| {
            activity.active()
                && !(self.one_shot
                    && matches!(self.state(), ContainerState::Exited | ContainerState::Dead))
                && activity.action != "create_instance"
        }) {
            result.status = activity.status();
            result.label = result.status.label().into();
            result.severity = Severity::Info;
            result.busy = true;
            if status == Status::Unhealthy && result.detail.is_none() {
                result = result.detail("unhealthy", Severity::Error);
            }
            if self.runtime.stale {
                let detail = result
                    .detail
                    .take()
                    .map(|detail| format!("{detail} · observations stale"))
                    .unwrap_or_else(|| "observations stale".into());
                let severity = if result.detail_severity == Severity::Error {
                    Severity::Error
                } else {
                    Severity::Warning
                };
                result = result.detail(detail, severity);
            }
        }
        if self.runtime.unexpected && result.detail.is_none() {
            result = result.detail("outside launch topology", Severity::Warning);
        }
        result
    }
}

impl Instance {
    fn mutable(&self) -> bool {
        !self.pending
            && !self.runtime.stale
            && self
                .runtime
                .activity
                .as_ref()
                .is_none_or(|activity| !activity.active())
    }

    pub fn can_start(&self) -> bool {
        self.mutable()
            && !self.services.iter().any(|service| {
                matches!(
                    service.state(),
                    ContainerState::Paused | ContainerState::Dead
                )
            })
            && self.services.iter().any(|service| {
                matches!(
                    service.state(),
                    ContainerState::Created | ContainerState::Exited | ContainerState::Missing
                )
            })
    }

    pub fn can_stop(&self) -> bool {
        self.mutable()
            && self.services.iter().any(|service| {
                matches!(
                    service.state(),
                    ContainerState::Running | ContainerState::Paused | ContainerState::Restarting
                )
            })
    }

    pub fn can_restart(&self) -> bool {
        self.mutable()
            && self.services.iter().any(|service| !service.one_shot)
            && self
                .services
                .iter()
                .filter(|service| !service.one_shot)
                .all(|service| service.can_restart())
    }

    pub fn suppress_resources(&self) -> bool {
        self.pending
            || self
                .runtime
                .activity
                .as_ref()
                .is_some_and(|activity| activity.active() && activity.action == "create_instance")
    }

    pub fn status_summary(&self) -> StatusSummary {
        let running: Vec<_> = self
            .services
            .iter()
            .filter(|service| !service.one_shot && !service.runtime.unexpected)
            .collect();
        let jobs: Vec<_> = self
            .services
            .iter()
            .filter(|service| service.one_shot && !service.runtime.unexpected)
            .collect();
        let count = running.iter().filter(|service| service.ready()).count();
        let active_jobs = jobs.iter().any(|service| {
            matches!(
                service.state(),
                ContainerState::Running | ContainerState::Paused | ContainerState::Restarting
            )
        });
        let all =
            |state| !running.is_empty() && running.iter().all(|service| service.state() == state);
        let complete = jobs
            .iter()
            .all(|service| service.status_summary().status == Status::Completed);
        let status = if self.runtime.stale {
            Status::Stale
        } else if self.workspace_only {
            if self.runtime.workspace_ready {
                Status::WorkspaceReady
            } else {
                Status::Failed
            }
        } else if !self.runtime.topology_known
            || running.is_empty() && jobs.is_empty()
            || self.services.iter().any(|service| {
                service.state() == ContainerState::Unknown
                    || service.state() == ContainerState::Running
                        && service.health_state() == HealthState::Unknown
            })
        {
            Status::Unknown
        } else if self
            .services
            .iter()
            .any(|service| service.runtime.unexpected)
        {
            Status::Degraded
        } else if running.is_empty() {
            job_status(&jobs)
        } else if !active_jobs
            && (all(ContainerState::Exited)
                || self.runtime.whole_stop
                    && running.iter().all(|service| {
                        matches!(
                            service.state(),
                            ContainerState::Exited | ContainerState::Created
                        )
                    }))
        {
            Status::Stopped
        } else if !active_jobs && all(ContainerState::Paused) {
            Status::Paused
        } else if !active_jobs && all(ContainerState::Created) {
            Status::NotStarted
        } else if all(ContainerState::Running)
            && complete
            && running
                .iter()
                .all(|service| service.health_state() == HealthState::Healthy)
        {
            Status::Healthy
        } else if all(ContainerState::Running)
            && complete
            && running.iter().all(|service| {
                !matches!(
                    service.health_state(),
                    HealthState::Unhealthy | HealthState::Unknown
                )
            })
        {
            Status::Running
        } else {
            Status::Degraded
        };
        let mut result = StatusSummary::new(status);
        result.running = count;
        result.expected = running.len();
        let issue = self
            .services
            .iter()
            .filter_map(|service| {
                let summary = service.status_summary();
                if matches!(
                    summary.status,
                    Status::Unhealthy
                        | Status::Failed
                        | Status::Interrupted
                        | Status::ContainerError
                        | Status::Missing
                ) || matches!(summary.detail_severity, Severity::Warning | Severity::Error)
                {
                    Some((
                        format!(
                            "{}: {}{}",
                            service.name,
                            summary.label,
                            summary
                                .detail
                                .as_ref()
                                .map(|detail| format!(" · {detail}"))
                                .unwrap_or_default()
                        ),
                        if summary.detail_severity == Severity::Error
                            || summary.severity == Severity::Error
                        {
                            Severity::Error
                        } else {
                            Severity::Warning
                        },
                    ))
                } else {
                    None
                }
            })
            .max_by_key(|(_, severity)| matches!(severity, Severity::Error));
        if let Some((issue, severity)) = issue {
            result = result.detail(issue, severity);
        } else if status == Status::Degraded {
            if let Some(service) = self
                .services
                .iter()
                .find(|service| !service.ready() && service.status_summary().busy)
                .or_else(|| self.services.iter().find(|service| !service.ready()))
            {
                result = result.detail(
                    format!("{}: {}", service.name, service.status_summary().label),
                    Severity::Warning,
                );
            }
        }
        if let Some(issue) = &self.runtime.issue {
            let detail = result
                .detail
                .take()
                .map(|detail| format!("{detail} · last operation: {issue}"))
                .unwrap_or_else(|| format!("last operation: {issue}"));
            let severity = if result.detail_severity == Severity::Error {
                Severity::Error
            } else {
                Severity::Warning
            };
            result = result.detail(detail, severity);
        }
        if !self.runtime.topology_known && !self.pending && result.detail.is_none() {
            result = result.detail(
                "topology unrecorded; start instance to capture configuration",
                Severity::Warning,
            );
        }
        if self.pending {
            result.status = Status::Creating;
            result.label = "Creating".into();
            result.severity = Severity::Info;
            result.busy = true;
        }
        if let Some(activity) = self
            .runtime
            .activity
            .as_ref()
            .filter(|activity| activity.active())
        {
            result.status = if self.services.is_empty() && activity.action == "create_instance" {
                Status::Creating
            } else {
                activity.status()
            };
            result.label = match &activity.service {
                Some(service) => format!("{} {service}", result.status.label()),
                None => result.status.label().into(),
            };
            result.severity = Severity::Info;
            result.busy = true;
            if self.runtime.stale {
                let detail = result
                    .detail
                    .take()
                    .map(|detail| format!("{detail} · observations stale"))
                    .unwrap_or_else(|| "observations stale".into());
                let severity = if result.detail_severity == Severity::Error {
                    Severity::Error
                } else {
                    Severity::Warning
                };
                result = result.detail(detail, severity);
            }
        }
        result
    }
}

fn job_status(jobs: &[&InstanceService]) -> Status {
    let statuses: Vec<_> = jobs.iter().map(|job| job.status_summary().status).collect();
    for status in [Status::Unknown, Status::Failed, Status::Interrupted] {
        if statuses.contains(&status) {
            return status;
        }
    }
    if statuses.iter().any(|status| {
        matches!(
            status,
            Status::Missing | Status::ContainerError | Status::Unhealthy
        )
    }) {
        return Status::Degraded;
    }
    if statuses.iter().all(|status| *status == Status::Completed) {
        return Status::Completed;
    }
    if jobs
        .iter()
        .any(|job| job.state() == ContainerState::Running)
    {
        return Status::Running;
    }
    let unfinished: Vec<_> = statuses
        .into_iter()
        .filter(|status| *status != Status::Completed)
        .collect();
    if unfinished.iter().all(|status| *status == Status::Paused) {
        Status::Paused
    } else if unfinished.iter().all(|status| *status == Status::Waiting) {
        Status::Waiting
    } else {
        Status::Degraded
    }
}

#[cfg(test)]
#[path = "tests/status.rs"]
mod tests;
