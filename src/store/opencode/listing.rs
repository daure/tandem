use serde::Serialize;

use super::{Activity, Pane, Snapshot, workspace_owner};
use crate::store::environments::Instance;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Availability {
    Observed,
    Unverified,
    WorkspaceMissing,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListedSession {
    pub session_id: String,
    pub instance: Option<String>,
    pub title: String,
    pub directory: String,
    pub server: String,
    pub activity: Activity,
    pub attached: bool,
    pub panes: Vec<Pane>,
    pub stale: bool,
    pub availability: Availability,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionListing {
    pub sessions: Vec<ListedSession>,
    pub observed_at_unix_seconds: u64,
    pub history_window_per_directory: usize,
    pub observation_error: Option<String>,
    pub inventory_error: Option<String>,
}

pub(crate) fn project(
    snapshot: Snapshot,
    instances: &[Instance],
    filter: Option<&str>,
    include_closed: bool,
) -> Vec<ListedSession> {
    let mut sessions = Vec::new();
    for session in snapshot.sessions {
        let active = session.attached()
            || (!session.stale
                && matches!(session.activity, Activity::Busy | Activity::AwaitingAnswer));
        if !include_closed && !active {
            continue;
        }
        let instance = workspace_owner(
            &session.directory,
            instances
                .iter()
                .map(|instance| (instance.name.as_str(), instance.workspace.as_str())),
        );
        if filter.is_some_and(|filter| instance != Some(filter)) {
            continue;
        }
        let missing = snapshot
            .observation
            .missing_directories
            .contains(&session.directory);
        sessions.push(ListedSession {
            session_id: session.id,
            instance: instance.map(str::to_owned),
            title: session.title,
            directory: session.directory,
            server: session.server,
            activity: if session.stale {
                Activity::Unknown
            } else {
                session.activity
            },
            attached: !session.panes.is_empty(),
            panes: session.panes,
            stale: session.stale,
            availability: if missing {
                Availability::WorkspaceMissing
            } else if session.stale {
                Availability::Unverified
            } else {
                Availability::Observed
            },
        });
    }
    sessions.sort_by(|left, right| {
        (&left.instance, &left.session_id, &left.server).cmp(&(
            &right.instance,
            &right.session_id,
            &right.server,
        ))
    });
    sessions
}

#[cfg(test)]
#[path = "tests/listing.rs"]
mod tests;
