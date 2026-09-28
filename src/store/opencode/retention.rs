use std::collections::BTreeMap;

use super::Snapshot;

const STALE_OBSERVATION_LIMIT: u8 = 3;

#[derive(Default)]
pub(crate) struct Retention {
    sessions: BTreeMap<String, u8>,
    clients: BTreeMap<(String, u32), u8>,
}

impl Retention {
    pub fn observe(&mut self, snapshot: &Snapshot) {
        self.sessions = snapshot
            .sessions
            .iter()
            .filter(|session| session.stale)
            .map(|session| {
                let attempts = self.sessions.get(&session.id).copied().unwrap_or(0);
                (session.id.clone(), attempts.saturating_add(1))
            })
            .collect();
        self.clients = snapshot
            .clients
            .iter()
            .filter(|client| client.stale)
            .map(|client| {
                let key = (client.pane.session.clone(), client.pane.id);
                let attempts = self.clients.get(&key).copied().unwrap_or(0);
                (key, attempts.saturating_add(1))
            })
            .collect();
    }

    pub fn visible_snapshot(&self, snapshot: &Snapshot) -> Snapshot {
        let mut snapshot = snapshot.clone();
        snapshot.sessions.retain(|session| {
            !session.stale
                || self.sessions.get(&session.id).copied().unwrap_or(0) < STALE_OBSERVATION_LIMIT
        });
        snapshot.clients.retain(|client| {
            !client.stale
                || self
                    .clients
                    .get(&(client.pane.session.clone(), client.pane.id))
                    .copied()
                    .unwrap_or(0)
                    < STALE_OBSERVATION_LIMIT
        });
        snapshot.resources.retain(|resource| {
            if resource.session_id.is_empty() {
                resource.pane_id.is_none_or(|pane_id| {
                    self.clients
                        .get(&(resource.zellij_session.clone(), pane_id))
                        .copied()
                        .unwrap_or(0)
                        < STALE_OBSERVATION_LIMIT
                })
            } else {
                self.sessions
                    .get(&resource.session_id)
                    .copied()
                    .unwrap_or(0)
                    < STALE_OBSERVATION_LIMIT
            }
        });
        snapshot
    }
}
