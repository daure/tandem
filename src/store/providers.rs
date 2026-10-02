use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub name: String,
    pub profile: String,
    pub description: String,
    pub protocol: String,
    #[serde(default)]
    pub feedback: Vec<String>,
}

impl Manifest {
    pub(crate) fn validate(&self) -> Result<(), String> {
        crate::store::environments::validate_name(&self.name)?;
        if self.schema_version != 1
            || self.protocol != "tandem-events-v1"
            || !["message", "ticket", "system_event", "generic"].contains(&self.profile.as_str())
        {
            return Err(
                "provider requires schema_version 1, tandem-events-v1, and a supported profile"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    NotStarted,
    Running,
    Stopped,
    Paused,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Start,
    Stop,
    Pause,
    Resume,
    Restart,
    Logs,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RuntimeObservation {
    pub status: Status,
    pub container_id: String,
}

#[derive(Debug)]
pub(crate) struct ActionOutcome {
    pub message: String,
    pub runtime: Option<RuntimeObservation>,
}

impl Action {
    pub(crate) fn unavailable_reason(
        self,
        status: Status,
        has_container: bool,
    ) -> Option<&'static str> {
        if status == Status::Unknown {
            return Some("Provider runtime state is unknown; wait for a successful refresh");
        }
        if self != Self::Start && !has_container {
            return Some("Provider has no collector container; use Start first");
        }
        match (self, status) {
            (Self::Start, Status::Paused) => Some("Provider is paused; use Resume"),
            (Self::Stop, Status::Stopped) => {
                Some("Provider is already stopped; use Start to run it")
            }
            (Self::Pause, Status::Paused) => {
                Some("Provider is already paused; use Resume to continue")
            }
            (Self::Resume, Status::Running) => {
                Some("Provider is already running; Pause it before using Resume")
            }
            (Self::Pause, status) if status != Status::Running => {
                Some("Only a running provider can be paused; use Start to run it")
            }
            (Self::Resume, status) if status != Status::Paused => {
                Some("Only a paused provider can be resumed; use Start for a stopped provider")
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ActionError {
    Unavailable(&'static str),
    Failed(String),
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => formatter.write_str(reason),
            Self::Failed(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for ActionError {}

impl From<String> for ActionError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<&str> for ActionError {
    fn from(error: &str) -> Self {
        Self::Failed(error.into())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub(crate) struct Provider {
    pub name: String,
    pub directory: String,
    pub manifest: Option<Manifest>,
    pub available: bool,
    pub status: Status,
    pub container_id: Option<String>,
    pub error: Option<String>,
    pub operation: Option<Action>,
}

impl Provider {
    pub(crate) fn action_unavailable(&self, action: Action) -> Option<&'static str> {
        if self.operation.is_some() {
            return Some("A provider operation is already in progress; wait for it to finish");
        }
        if let Some(reason) = action.unavailable_reason(self.status, self.container_id.is_some()) {
            return Some(reason);
        }
        if action == Action::Start && self.status == Status::Running {
            return Some("Provider is already running; use Stop to stop it");
        }
        if action == Action::Start && self.status != Status::Running && !self.available {
            return Some("Provider package is unavailable; restore a valid package before Start");
        }
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub(crate) struct Snapshot {
    pub providers: Vec<Provider>,
    pub error: Option<String>,
}

impl Snapshot {
    pub(crate) fn start_stop_action(&self) -> Action {
        if !self.providers.is_empty()
            && self
                .providers
                .iter()
                .all(|provider| matches!(provider.status, Status::Running | Status::Paused))
        {
            Action::Stop
        } else {
            Action::Start
        }
    }

    pub(crate) fn pause_resume_action(&self) -> Action {
        if !self.providers.is_empty()
            && self
                .providers
                .iter()
                .all(|provider| provider.status == Status::Paused)
        {
            Action::Resume
        } else {
            Action::Pause
        }
    }

    pub(crate) fn action_targets(&self, action: Action) -> Vec<String> {
        self.providers
            .iter()
            .filter(|provider| provider.action_unavailable(action).is_none())
            .map(|provider| provider.name.clone())
            .collect()
    }
}
