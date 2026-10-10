use std::collections::{HashMap, HashSet};

use super::{Row, Snapshot, Tone, workspace_owner};
use crate::store::opencode::resources::Owner;

pub(super) fn known_instances<'a>(snapshot: &Snapshot, owners: &'a [Owner]) -> HashSet<&'a str> {
    snapshot
        .workspace_directories()
        .filter_map(|directory| owner(directory, owners))
        .collect()
}

pub(super) fn active_first(rows: &mut [Row], snapshot: &Snapshot, owners: &[Owner]) {
    let active_directories = snapshot
        .sessions
        .iter()
        .filter(|session| session.attached())
        .map(|session| session.directory.as_str())
        .chain(
            snapshot
                .clients
                .iter()
                .map(|client| client.directory.as_str()),
        )
        .collect::<HashSet<_>>();
    let active_instances = active_directories
        .iter()
        .filter_map(|directory| owner(directory, owners))
        .collect::<HashSet<_>>();
    let mut directory_positions = HashMap::new();
    let mut instance_positions = HashMap::new();
    for (directory, pane) in snapshot
        .sessions
        .iter()
        .flat_map(|session| {
            session
                .panes
                .iter()
                .map(move |pane| (session.directory.as_str(), pane))
        })
        .chain(
            snapshot
                .clients
                .iter()
                .map(|client| (client.directory.as_str(), &client.pane)),
        )
    {
        let Some(position) = snapshot
            .zellij_tabs
            .get(&pane.session)
            .and_then(|tabs| tabs.get(&pane.tab_id))
            .map(|position| (pane.session.as_str(), *position))
        else {
            continue;
        };
        directory_positions
            .entry(directory)
            .and_modify(|current| *current = std::cmp::min(*current, position))
            .or_insert(position);
        if let Some(instance) = owner(directory, owners) {
            instance_positions
                .entry(instance)
                .and_modify(|current| *current = std::cmp::min(*current, position))
                .or_insert(position);
        }
    }
    let mut ranks = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let active = if let Some(instance) = row.instance.as_deref() {
            active_instances.contains(instance)
        } else {
            row.workspace
                .as_deref()
                .is_some_and(|directory| active_directories.contains(directory))
        };
        let group = match (active, row.instance.is_some()) {
            (true, false) => 0,
            (true, true) => 1,
            (false, true) => 2,
            (false, false) => 3,
        };
        let position = if let Some(instance) = row.instance.as_deref() {
            instance_positions.get(instance)
        } else {
            row.workspace
                .as_deref()
                .and_then(|directory| directory_positions.get(directory))
        }
        .copied();
        let rank = row
            .parent
            .as_ref()
            .and_then(|parent| ranks.get(parent))
            .copied()
            .unwrap_or((group, position.is_none(), position, index));
        ranks.insert(row.id.clone(), rank);
    }
    rows.sort_by_key(|row| ranks[&row.id]);
}

pub(super) fn active_templates_first(rows: &mut [Row]) {
    let active_templates = rows
        .iter()
        .filter(|row| row.instance.is_some() && row.opencode.is_none() && row.tone == Tone::Success)
        .map(|row| row.directory.clone())
        .collect::<HashSet<_>>();
    let mut ranks = HashMap::new();
    for (index, row) in rows.iter_mut().enumerate() {
        if row.is_template() && active_templates.contains(row.directory.as_str()) {
            row.tone = Tone::Success;
        }
        let active = row.is_template() && row.tone == Tone::Success;
        let rank = row
            .parent
            .as_ref()
            .and_then(|parent| ranks.get(parent))
            .copied()
            .unwrap_or((!active, index));
        ranks.insert(row.id.clone(), rank);
    }
    rows.sort_by_key(|row| ranks[&row.id]);
}

fn owner<'a>(directory: &str, owners: &'a [Owner]) -> Option<&'a str> {
    workspace_owner(
        directory,
        owners
            .iter()
            .map(|owner| (owner.name.as_str(), owner.workspace.as_str())),
    )
}

pub(super) fn apply_session_pings(
    rows: &mut [Row],
    snapshot: &crate::store::environments::EnvironmentSnapshot,
    owners: &[Owner],
) {
    for row in rows {
        let Some(super::Target::Session { id, .. }) = &row.opencode else {
            continue;
        };
        let Some(instance) = row
            .workspace
            .as_deref()
            .and_then(|directory| owner(directory, owners))
        else {
            continue;
        };
        row.ping_revision = snapshot
            .session_pings
            .iter()
            .find(|ping| ping.instance == instance && ping.session_id == *id)
            .map(|ping| ping.revision);
    }
}
