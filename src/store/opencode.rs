use std::path::Path;

use serde::{Deserialize, Serialize};

pub(crate) mod conversation;
pub(crate) mod resources;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Activity {
    Busy,
    AwaitingAnswer,
    Idle,
    #[default]
    Unknown,
}

impl Activity {
    pub fn completed(self) -> bool {
        matches!(self, Self::Idle | Self::AwaitingAnswer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct Pane {
    pub session: String,
    pub id: u32,
    pub tab_id: u32,
    pub tab_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CloseScope {
    Instance(String),
    Directory(String),
    ExternalWorkspaces,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct Session {
    pub id: String,
    pub title: String,
    pub directory: String,
    pub server: String,
    pub activity: Activity,
    pub activity_started_at_milliseconds: Option<u64>,
    pub activity_elapsed_milliseconds: Option<u64>,
    pub agent: Option<String>,
    pub agent_color: Option<String>,
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub provider_name: Option<String>,
    pub variant: Option<String>,
    pub context_tokens: Option<u64>,
    pub context_limit: Option<u64>,
    pub panes: Vec<Pane>,
    pub updated: u64,
    pub last_question: Option<String>,
    pub question_observed: bool,
    #[serde(skip)]
    pub approval_pending: Option<bool>,
    pub stale: bool,
}

impl Session {
    pub fn attached(&self) -> bool {
        !self.panes.is_empty()
    }

    pub fn live(&self) -> bool {
        self.attached() || matches!(self.activity, Activity::Busy | Activity::AwaitingAnswer)
    }

    pub fn saved(&self) -> bool {
        !self.attached() && self.activity == Activity::Idle && !self.stale
    }

    pub fn label(&self) -> &'static str {
        match (self.attached(), self.activity) {
            (true, Activity::Busy) => "attached · busy",
            (true, Activity::AwaitingAnswer) => "attached · awaiting answer",
            (false, Activity::AwaitingAnswer) => "detached · awaiting answer",
            (true, Activity::Idle) => "attached · idle",
            (false, Activity::Busy) => "detached · busy",
            (false, Activity::Idle) => "saved",
            (true, Activity::Unknown) => "attached · activity unknown",
            (false, Activity::Unknown) => "attachment/activity unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Client {
    pub title: String,
    pub directory: String,
    pub server: String,
    pub pane: Pane,
    pub stale: bool,
    pub awaiting_presence_since: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub directories: Vec<String>,
    pub sessions: Vec<Session>,
    pub clients: Vec<Client>,
    pub resources: Vec<resources::ProcessResource>,
    pub error: Option<String>,
}

impl Snapshot {
    pub fn workspace_directories(&self) -> impl Iterator<Item = &str> {
        self.directories
            .iter()
            .map(String::as_str)
            .chain(
                self.sessions
                    .iter()
                    .map(|session| session.directory.as_str()),
            )
            .chain(self.clients.iter().map(|client| client.directory.as_str()))
    }

    pub fn completed_since(&self, previous: &Self) -> bool {
        self.sessions.iter().any(|session| {
            !session.stale
                && session.activity.completed()
                && previous.sessions.iter().any(|candidate| {
                    candidate.id == session.id
                        && !candidate.stale
                        && candidate.activity == Activity::Busy
                })
        })
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    pub live: usize,
    pub attached: usize,
    pub busy: usize,
}

impl Counts {
    pub fn of<'a>(sessions: impl Iterator<Item = &'a Session>) -> Self {
        sessions.fold(Self::default(), |mut counts, session| {
            counts.live += usize::from(session.live());
            counts.attached += usize::from(session.attached());
            counts.busy += usize::from(session.activity == Activity::Busy);
            counts
        })
    }
}

pub(crate) fn workspace_owner<'a>(
    directory: &str,
    workspaces: impl Iterator<Item = (&'a str, &'a str)>,
) -> Option<&'a str> {
    if !Path::new(directory).is_absolute()
        || Path::new(directory)
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return None;
    }
    workspaces
        .filter(|(_, workspace)| {
            !workspace.is_empty() && Path::new(directory).starts_with(workspace)
        })
        .max_by_key(|(_, workspace)| Path::new(workspace).components().count())
        .map(|(name, _)| name)
}

#[cfg(test)]
#[path = "tests/opencode.rs"]
mod tests;
