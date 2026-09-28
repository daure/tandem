use std::collections::{HashMap, HashSet};

use super::{Row, Snapshot, workspace_owner};
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
        let rank = row
            .parent
            .as_ref()
            .and_then(|parent| ranks.get(parent))
            .copied()
            .unwrap_or((group, index));
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
