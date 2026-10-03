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
    #[serde(default)]
    pub streams: Vec<String>,
    #[serde(default)]
    pub stream_control: bool,
}

impl Manifest {
    pub(crate) fn validate(&self) -> Result<(), String> {
        crate::store::environments::validate_name(&self.name)?;
        let mut streams = std::collections::BTreeSet::new();
        for stream in &self.streams {
            validate_stream(stream)?;
            if !streams.insert(stream) {
                return Err("provider stream names must be unique".into());
            }
        }
        if self.stream_control && self.streams.is_empty() {
            return Err("stream control requires declared streams".into());
        }
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
    Starting,
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

pub(crate) struct StreamActionOutcome {
    pub message: String,
    pub runtime: RuntimeObservation,
    pub streams: Vec<Stream>,
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
            (Self::Stop, Status::Stopped) => {
                Some("Provider is already stopped; use Start to run it")
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
    #[serde(default)]
    pub streams: Vec<Stream>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub(crate) struct Stream {
    pub name: String,
    pub controllable: bool,
    pub enabled: bool,
    pub status: Status,
    pub operation: Option<Action>,
    pub error: Option<String>,
    pub total: u64,
    pub handovers: u64,
}

impl Stream {
    pub(crate) fn start_stop_action(&self, provider: &Provider) -> Action {
        if provider.status == Status::Running && self.enabled {
            Action::Stop
        } else {
            Action::Start
        }
    }

    pub(crate) fn action_unavailable(
        &self,
        provider: &Provider,
        action: Action,
    ) -> Option<&'static str> {
        if !self.controllable {
            return Some("Provider does not support controls for this stream");
        }
        if provider.operation.is_some()
            || self.operation.is_some()
            || provider
                .streams
                .iter()
                .any(|stream| stream.operation.is_some())
        {
            return Some("A provider or stream operation is already in progress");
        }
        if action == Action::Logs {
            return Some("Logs belong to the provider collector");
        }
        if provider.status == Status::Unknown {
            return Some("Provider runtime state is unknown; wait for a successful refresh");
        }
        if action == Action::Start
            && matches!(provider.status, Status::Stopped | Status::NotStarted)
            && !provider.available
        {
            return Some("Provider package is unavailable; restore a valid package before Start");
        }
        if action == Action::Stop && provider.status != Status::Running {
            return Some("Stream is already stopped or its provider is paused");
        }
        match (action, self.status) {
            (Action::Start, Status::Running | Status::Starting)
                if provider.status == Status::Running =>
            {
                Some("Stream is already running or starting")
            }
            (Action::Stop, Status::Stopped) => Some("Stream is already stopped"),
            _ => None,
        }
    }
}

pub(crate) fn validate_stream(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        return Err("stream names require 1–80 bytes without control characters".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamControl {
    pub stream: String,
    pub enabled: bool,
    pub revision: i64,
}

impl Provider {
    pub(crate) fn action_unavailable(&self, action: Action) -> Option<&'static str> {
        if self.operation.is_some() || self.streams.iter().any(|stream| stream.operation.is_some())
        {
            return Some("A provider operation is already in progress; wait for it to finish");
        }
        if let Some(reason) = action.unavailable_reason(self.status, self.container_id.is_some()) {
            return Some(reason);
        }
        if action == Action::Start
            && self.status == Status::Running
            && !self
                .streams
                .iter()
                .any(|stream| stream.controllable && !stream.enabled)
        {
            return Some("Provider is already running; use Stop to stop it");
        }
        if action == Action::Start
            && !matches!(self.status, Status::Running | Status::Paused)
            && !self.available
        {
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
        if self
            .providers
            .iter()
            .any(|provider| matches!(provider.status, Status::Running | Status::Paused))
        {
            Action::Stop
        } else {
            Action::Start
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
