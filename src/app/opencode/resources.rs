use super::{Row, Snapshot, Target, client_row_id, workspace_owner};
use crate::app::{details, properties::Property};
use crate::store::opencode::resources::{Owner, Totals};

pub(super) fn apply(rows: &mut [Row], snapshot: &Snapshot, owners: &[Owner]) {
    let totals = Totals::new(snapshot, owners);
    let clients: std::collections::BTreeMap<_, _> = snapshot
        .clients
        .iter()
        .filter_map(|client| {
            let scope = workspace_owner(
                &client.directory,
                owners
                    .iter()
                    .map(|owner| (owner.name.as_str(), owner.workspace.as_str())),
            )?;
            Some((client_row_id(scope, client), &client.pane))
        })
        .collect();
    let errors: Vec<_> = snapshot
        .resources
        .iter()
        .filter(|process| process.error.is_some())
        .collect();
    for row in rows {
        let usage = if let Some(instance) = &row.instance {
            totals.instances.get(instance)
        } else if row.is_template() {
            totals.templates.get(&row.directory)
        } else if let Some(instance) = row.id.strip_prefix("sessions:") {
            totals.instances.get(instance)
        } else if row.id == "opencode-workspaces" {
            Some(&totals.external)
        } else if let Some(directory) = row.id.strip_prefix("opencode-workspace:") {
            totals.workspaces.get(directory)
        } else if let Some(Target::Session { id, pane, .. }) = &row.opencode {
            if row
                .parent
                .as_ref()
                .is_some_and(|parent| parent.starts_with("opencode:"))
            {
                pane.as_ref()
                    .and_then(|pane| totals.panes.get(&(pane.session.clone(), pane.id)))
            } else {
                totals.sessions.get(id)
            }
        } else if let Some(Target::Client { pane }) = &row.opencode {
            totals.panes.get(&(pane.session.clone(), pane.id))
        } else {
            clients
                .get(&row.id)
                .and_then(|pane| totals.panes.get(&(pane.session.clone(), pane.id)))
        };
        let Some(usage) = usage else { continue };
        if *usage == Default::default() {
            continue;
        }
        row.metrics.merge(usage);
        row.hide_resources = false;
        for property in &mut row.details {
            property.name = match property.name.as_str() {
                "Memory limit" => "Container memory limit".into(),
                "Resource refresh" => "Container resource refresh".into(),
                "Resource sample (Unix)" => "Container resource sample (Unix)".into(),
                _ => continue,
            };
        }
        for name in ["Memory", "CPU"] {
            if !row.details.iter().any(|property| property.name == name) {
                row.details.push(Property::new(name, "—"));
            }
        }
        row.details.retain(|property| {
            !matches!(
                property.name.as_str(),
                "Resource sample age" | "Resource coverage"
            )
        });
        details::usage_details(&mut row.details, &row.metrics);
        for process in &errors {
            let matches_pane = |pane: &super::Pane| {
                process.pane_id == Some(pane.id) && process.zellij_session == pane.session
            };
            let matches = match &row.opencode {
                Some(Target::Session { id, .. }) => &process.session_id == id,
                Some(Target::Client { pane }) => matches_pane(pane),
                Some(Target::Workspace) => false,
                None => clients.get(&row.id).is_some_and(|pane| matches_pane(pane)),
            };
            if matches && let Some(error) = &process.error {
                row.details
                    .push(Property::new("Client resource error", error).tone(super::Tone::Warning));
            }
        }
        row.details.push(Property::new(
            "Client resource scope",
            "Client-process RSS and CPU; shared servers and child processes are excluded",
        ));
        row.details.push(Property::new(
            "Client resource refresh",
            "Background observation, at most once every 2s; CPU is averaged between samples",
        ));
    }
}
