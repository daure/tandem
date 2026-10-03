use std::collections::BTreeSet;

use super::{Client, Session, Snapshot};

pub(crate) type PaneKey = (String, u32);
pub(crate) type DirectoryKey = (String, String);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Evidence {
    pub missing_directories: BTreeSet<String>,
    pub verified_panes: BTreeSet<PaneKey>,
    pub unfinished_sessions: BTreeSet<String>,
    pub failures: Vec<Failure>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Exclusions {
    pub servers: BTreeSet<String>,
    pub sessions: BTreeSet<String>,
    pub panes: BTreeSet<PaneKey>,
    pub directories: BTreeSet<DirectoryKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Global,
    Server(String),
    Directory(DirectoryKey),
    Pane(PaneKey),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub scope: Scope,
    pub message: String,
}

impl Failure {
    pub fn global(message: impl Into<String>) -> Self {
        Self {
            scope: Scope::Global,
            message: message.into(),
        }
    }

    pub fn server(server: &str, message: impl Into<String>) -> Self {
        Self {
            scope: Scope::Server(server.into()),
            message: message.into(),
        }
    }

    pub fn directory(server: &str, directory: &str, message: impl Into<String>) -> Self {
        Self {
            scope: Scope::Directory((server.into(), directory.into())),
            message: message.into(),
        }
    }

    pub fn pane(session: &str, id: u32, message: impl Into<String>) -> Self {
        Self {
            scope: Scope::Pane((session.into(), id)),
            message: message.into(),
        }
    }
}

impl Snapshot {
    pub fn missing_directory(&self, directory: &str) -> bool {
        self.observation.missing_directories.contains(directory)
    }

    pub fn unfinished_missing_workspace(&self, session: &Session) -> bool {
        self.missing_directory(&session.directory)
            && self.observation.unfinished_sessions.contains(&session.id)
    }

    pub fn uncertain_session(&self, session: &Session) -> bool {
        session.stale
            && !session.panes.iter().any(|pane| {
                self.observation
                    .verified_panes
                    .contains(&(pane.session.clone(), pane.id))
            })
            && !self.unfinished_missing_workspace(session)
    }

    pub fn uncertain_client(&self, client: &Client) -> bool {
        client.stale
            && !self
                .observation
                .verified_panes
                .contains(&(client.pane.session.clone(), client.pane.id))
    }

    pub fn observation_error(&self) -> Option<String> {
        let messages = self
            .observation
            .failures
            .iter()
            .map(|failure| failure.message.as_str())
            .collect::<BTreeSet<_>>();
        (!messages.is_empty()).then(|| messages.into_iter().collect::<Vec<_>>().join("\n"))
    }
}
