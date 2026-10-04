use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use super::{
    Snapshot,
    observation::{Exclusions, PaneKey, Scope},
};

const RETRY_LIMIT: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Session(String, String, String),
    Client(PaneKey, String, String),
    Directory(String, String),
    Server(String),
    Pane(PaneKey),
}

struct Retry {
    attempts: u8,
    next: Instant,
    panes: Vec<PaneKey>,
}

#[derive(Default)]
pub(crate) struct Retention {
    retries: BTreeMap<Source, Retry>,
}

impl Retention {
    pub fn rediscover(&mut self) {
        self.retries.retain(|_, retry| retry.attempts < RETRY_LIMIT);
    }

    pub fn observe(&mut self, snapshot: &Snapshot) -> Vec<String> {
        self.observe_at(snapshot, Instant::now())
    }

    fn observe_at(&mut self, snapshot: &Snapshot, now: Instant) -> Vec<String> {
        let mut uncertain = BTreeMap::new();
        for session in snapshot
            .sessions
            .iter()
            .filter(|session| snapshot.uncertain_session(session))
        {
            uncertain.insert(
                Source::Session(
                    session.id.clone(),
                    session.server.clone(),
                    session.directory.clone(),
                ),
                session
                    .panes
                    .iter()
                    .map(|pane| (pane.session.clone(), pane.id))
                    .collect(),
            );
        }
        for client in snapshot
            .clients
            .iter()
            .filter(|client| snapshot.uncertain_client(client))
        {
            uncertain.insert(
                Source::Client(
                    (client.pane.session.clone(), client.pane.id),
                    client.server.clone(),
                    client.directory.clone(),
                ),
                vec![(client.pane.session.clone(), client.pane.id)],
            );
        }
        for failure in &snapshot.observation.failures {
            match &failure.scope {
                Scope::Directory((server, directory))
                    if !has_directory(snapshot, server, directory) =>
                {
                    uncertain.insert(
                        Source::Directory(server.clone(), directory.clone()),
                        Vec::new(),
                    );
                }
                Scope::Server(server) if !has_server(snapshot, server) => {
                    uncertain.insert(Source::Server(server.clone()), Vec::new());
                }
                Scope::Pane(pane) if !has_pane(snapshot, pane) => {
                    uncertain.insert(Source::Pane(pane.clone()), vec![pane.clone()]);
                }
                _ => {}
            }
        }
        self.retries.retain(|source, retry| {
            retry.attempts == RETRY_LIMIT || uncertain.contains_key(source)
        });
        let mut abandoned = Vec::new();
        for (source, panes) in uncertain {
            let retry = self.retries.entry(source.clone()).or_insert(Retry {
                attempts: 0,
                next: now,
                panes,
            });
            if retry.attempts == RETRY_LIMIT || now < retry.next {
                continue;
            }
            retry.attempts += 1;
            retry.next = now + Duration::from_secs(1 << (retry.attempts - 1));
            if retry.attempts == RETRY_LIMIT {
                abandoned.push(format!(
                    "Abandoned OpenCode observation for {source:?} after {} failed checks: {}",
                    retry.attempts,
                    snapshot
                        .error
                        .as_deref()
                        .unwrap_or("client or session could not be verified")
                ));
            }
        }
        abandoned
    }

    pub fn next_retry(&self) -> Option<Instant> {
        self.retries
            .values()
            .filter(|retry| retry.attempts < RETRY_LIMIT)
            .map(|retry| retry.next)
            .min()
    }

    pub fn exclusions(&self) -> Exclusions {
        let mut excluded = Exclusions::default();
        for (source, retry) in self
            .retries
            .iter()
            .filter(|(_, retry)| retry.attempts == RETRY_LIMIT)
        {
            excluded.panes.extend(retry.panes.iter().cloned());
            match source {
                Source::Session(id, server, directory) => {
                    excluded.sessions.insert(id.clone());
                    excluded
                        .directories
                        .insert((server.clone(), directory.clone()));
                }
                Source::Client(pane, server, directory) => {
                    excluded.panes.insert(pane.clone());
                    excluded
                        .directories
                        .insert((server.clone(), directory.clone()));
                }
                Source::Directory(server, directory) => {
                    excluded
                        .directories
                        .insert((server.clone(), directory.clone()));
                }
                Source::Server(server) => {
                    excluded.servers.insert(server.clone());
                }
                Source::Pane(pane) => {
                    excluded.panes.insert(pane.clone());
                }
            }
        }
        excluded
    }

    pub fn discard_abandoned(&self, snapshot: &mut Snapshot) {
        let excluded = self.exclusions();
        snapshot.sessions.retain(|session| {
            !excluded.sessions.contains(&session.id) && !excluded.servers.contains(&session.server)
        });
        snapshot.clients.retain(|client| {
            !excluded
                .panes
                .contains(&(client.pane.session.clone(), client.pane.id))
                && !excluded.servers.contains(&client.server)
        });
    }

    pub fn visible_snapshot(&self, snapshot: &Snapshot) -> Snapshot {
        let mut visible = snapshot.clone();
        visible.sessions.retain(|session| {
            !snapshot.uncertain_session(session)
                && (!snapshot.missing_directory(&session.directory)
                    || session.attached()
                    || session.activity != super::Activity::Idle
                    || snapshot.unfinished_missing_workspace(session))
        });
        visible
            .clients
            .retain(|client| !snapshot.uncertain_client(client));
        visible.directories.retain(|directory| {
            !snapshot.missing_directory(directory)
                || visible
                    .sessions
                    .iter()
                    .any(|session| session.directory == *directory)
                || visible
                    .clients
                    .iter()
                    .any(|client| client.directory == *directory)
        });
        visible.resources.retain(|resource| {
            if resource.session_id.is_empty() {
                visible.clients.iter().any(|client| {
                    resource.pane_id == Some(client.pane.id)
                        && resource.zellij_session == client.pane.session
                })
            } else {
                visible
                    .sessions
                    .iter()
                    .any(|session| session.id == resource.session_id)
            }
        });
        visible.observation.failures = snapshot
            .observation
            .failures
            .iter()
            .filter(|failure| match &failure.scope {
                Scope::Global => true,
                Scope::Server(server) => has_server(&visible, server),
                Scope::Directory((server, directory)) => has_directory(&visible, server, directory),
                Scope::Pane(key) => has_pane(&visible, key),
            })
            .cloned()
            .collect();
        if !snapshot.observation.failures.is_empty() {
            visible.error = visible.observation_error();
        } else if visible.sessions.is_empty()
            && visible.clients.is_empty()
            && (!snapshot.sessions.is_empty() || !snapshot.clients.is_empty())
        {
            visible.error = None;
        }
        visible
    }
}

fn has_server(snapshot: &Snapshot, server: &str) -> bool {
    snapshot
        .sessions
        .iter()
        .any(|session| session.server == server)
        || snapshot
            .clients
            .iter()
            .any(|client| client.server == server)
}

fn has_directory(snapshot: &Snapshot, server: &str, directory: &str) -> bool {
    snapshot
        .sessions
        .iter()
        .any(|session| session.server == server && session.directory == directory)
        || snapshot
            .clients
            .iter()
            .any(|client| client.server == server && client.directory == directory)
}

fn has_pane(snapshot: &Snapshot, key: &PaneKey) -> bool {
    snapshot
        .sessions
        .iter()
        .flat_map(|session| &session.panes)
        .chain(snapshot.clients.iter().map(|client| &client.pane))
        .any(|pane| pane.session == key.0 && pane.id == key.1)
}

#[cfg(test)]
#[path = "../tests/opencode_retention.rs"]
mod tests;
