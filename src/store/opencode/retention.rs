use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use super::Snapshot;

const STALE_GRACE: Duration = Duration::from_secs(6);

#[derive(Default)]
pub(crate) struct Retention {
    sessions: BTreeMap<String, Instant>,
    clients: BTreeMap<(String, u32), Instant>,
}

impl Retention {
    pub fn observe(&mut self, snapshot: &Snapshot) {
        self.sessions = snapshot
            .sessions
            .iter()
            .filter(|session| session.stale)
            .map(|session| {
                let since = self
                    .sessions
                    .get(&session.id)
                    .copied()
                    .unwrap_or_else(Instant::now);
                (session.id.clone(), since)
            })
            .collect();
        self.clients = snapshot
            .clients
            .iter()
            .filter(|client| client.stale)
            .map(|client| {
                let key = (client.pane.session.clone(), client.pane.id);
                let since = self.clients.get(&key).copied().unwrap_or_else(Instant::now);
                (key, since)
            })
            .collect();
    }

    pub fn visible_snapshot(&self, snapshot: &Snapshot) -> Snapshot {
        let mut snapshot = snapshot.clone();
        snapshot
            .sessions
            .retain(|session| !session.stale || within_grace(self.sessions.get(&session.id)));
        snapshot.clients.retain(|client| {
            !client.stale
                || within_grace(
                    self.clients
                        .get(&(client.pane.session.clone(), client.pane.id)),
                )
        });
        snapshot.resources.retain(|resource| {
            if resource.session_id.is_empty() {
                resource.pane_id.is_none_or(|pane_id| {
                    within_grace(
                        self.clients
                            .get(&(resource.zellij_session.clone(), pane_id)),
                    )
                })
            } else {
                within_grace(self.sessions.get(&resource.session_id))
            }
        });
        snapshot
    }
}

fn within_grace(since: Option<&Instant>) -> bool {
    since.is_none_or(|since| since.elapsed() < STALE_GRACE)
}

#[cfg(test)]
#[path = "../tests/opencode_retention.rs"]
mod tests;
