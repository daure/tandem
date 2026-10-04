use rusqlite::{Connection, TransactionBehavior, params};

use super::{RuleStore, acceptances, decode, encode};
use crate::store::{
    environments::EnvironmentSnapshot,
    events::Error,
    opencode::{Snapshot, workspace_owner},
    rules::AcceptanceWorkspace,
};

impl RuleStore {
    pub(super) fn workspaces(
        &self,
        connection: &Connection,
    ) -> Result<std::collections::BTreeMap<i64, AcceptanceWorkspace>, Error> {
        let mut statement = connection.prepare(
            "SELECT w.acceptance_id, w.payload FROM rule_acceptance_workspaces w
             JOIN rule_acceptances a ON a.id = w.acceptance_id
             JOIN event_attempts p ON p.id = a.attempt_id
             JOIN events e ON e.sequence = p.event_sequence WHERE e.namespace = ?1",
        )?;
        statement
            .query_map([&self.config.namespace], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|row| {
                let (id, text) = row?;
                Ok((id, decode(&text)?))
            })
            .collect()
    }

    pub(crate) fn remember_workspaces(
        &self,
        inventory: &EnvironmentSnapshot,
        observed: &Snapshot,
    ) -> Result<(), Error> {
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let remembered = self.workspaces(&transaction)?;
        for acceptance in acceptances(&transaction, None, &self.config.namespace)? {
            let existing = remembered.get(&acceptance.id);
            let instance = inventory.instances.iter().find(|instance| {
                instance.name == acceptance.instance
                    && instance.template == acceptance.rule.definition.template
            });
            let Some(instance) = instance else {
                continue;
            };
            if !super::super::startup::belongs_to(
                &self.config,
                &instance.name,
                &instance.template,
                acceptance.operation_id.as_deref(),
            )
            .map_err(Error::Storage)?
            {
                continue;
            }
            let directory = instance.workspace.clone();
            let mut workspace = existing.cloned().unwrap_or_default();
            // A recorded directory pins history to this acceptance even after instance removal.
            if !workspace.directory.is_empty() && workspace.directory != directory {
                continue;
            }
            workspace.directory = directory;
            for session in &observed.sessions {
                if session.stale
                    || workspace_owner(
                        &session.directory,
                        std::iter::once((
                            acceptance.instance.as_str(),
                            workspace.directory.as_str(),
                        )),
                    ) != Some(acceptance.instance.as_str())
                {
                    continue;
                }
                let mut saved = session.clone();
                saved.panes.clear();
                saved.tab_position = None;
                saved.activity = crate::store::opencode::Activity::Idle;
                saved.activity_started_at_milliseconds = None;
                saved.activity_elapsed_milliseconds = None;
                saved.approval_pending = None;
                if let Some(previous) = workspace.sessions.iter_mut().find(|row| row.id == saved.id)
                {
                    if saved.server.is_empty() {
                        saved.server.clone_from(&previous.server);
                    }
                    *previous = saved;
                } else {
                    workspace.sessions.push(saved);
                }
            }
            if existing != Some(&workspace) {
                transaction.execute(
                    "INSERT INTO rule_acceptance_workspaces(acceptance_id, payload) VALUES (?1, ?2)
                     ON CONFLICT(acceptance_id) DO UPDATE SET payload=excluded.payload",
                    params![acceptance.id, encode(&workspace)?],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/acceptance_workspaces.rs"]
mod tests;
