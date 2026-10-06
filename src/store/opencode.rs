use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

pub(crate) mod conversation;
pub(crate) mod observation;
pub(crate) mod resources;
pub(crate) mod retention;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Launch {
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
}

impl Launch {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(model) = &self.model {
            let (provider, path) = model
                .split_once('/')
                .ok_or("model must be provider/model, optionally followed by #variant")?;
            if provider.is_empty()
                || path.is_empty()
                || path.starts_with('#')
                || path.ends_with('#')
                || model.len() > 200
                || model.matches('#').count() > 1
                || !provider
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
                || !model
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.#:".contains(&byte))
            {
                return Err("model must be provider/model, optionally followed by #variant".into());
            }
            if let Some((_, embedded)) = model.split_once('#')
                && self
                    .variant
                    .as_deref()
                    .is_some_and(|variant| variant != embedded)
            {
                return Err("variant conflicts with the model's #variant".into());
            }
        }
        if let Some(variant) = &self.variant
            && (variant.is_empty()
                || variant.len() > 100
                || !variant
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-.:".contains(&byte)))
        {
            return Err("variant must contain 1 to 100 letters, digits, underscores, hyphens, dots or colons".into());
        }
        Ok(())
    }

    pub(crate) fn selector(&self) -> Option<String> {
        self.model.as_ref().map(|model| match &self.variant {
            Some(variant) if !model.contains('#') => format!("{model}#{variant}"),
            _ => model.clone(),
        })
    }
}

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
pub(crate) struct SessionLocation {
    pub session_id: Option<String>,
    pub pane: Pane,
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
    pub tab_position: Option<TabPosition>,
    pub updated: u64,
    pub last_question: Option<String>,
    pub question_observed: bool,
    #[serde(skip)]
    pub approval_pending: Option<bool>,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub(crate) struct TabPosition {
    pub zellij_session: String,
    pub pane_id: u32,
    pub index: usize,
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
    pub zellij_tabs: BTreeMap<String, BTreeMap<u32, usize>>,
    pub resources: Vec<resources::ProcessResource>,
    pub error: Option<String>,
    pub observation: observation::Evidence,
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
