use std::{
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::Store;
use crate::store::{
    environments::{Instance, RuntimeInventory},
    state_stream,
};
use rusqlite::TransactionBehavior;

#[derive(Deserialize, Serialize)]
struct InventoryObservation {
    at: u64,
    revision: i64,
    result: Result<Vec<Instance>, String>,
}

impl Store {
    pub(crate) fn save_opencode_observation(
        &self,
        snapshot: &crate::store::opencode::Snapshot,
    ) -> Result<std::collections::BTreeMap<(String, String), Option<String>>, String> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut snapshot = snapshot.clone();
        let edits = self.edits_in(&transaction)?;
        snapshot.apply_session_edits(&edits);
        self.save_observation_in(&transaction, "opencode", &snapshot)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(edits)
    }

    pub(crate) fn preserve_failed_inventory(
        &self,
        mut inventory: RuntimeInventory,
    ) -> Result<RuntimeInventory, String> {
        if inventory.runtime_error.is_none() {
            return Ok(inventory);
        }
        let previous = self.boundary(None)?.snapshot.instances;
        for value in previous["instances"].as_array().into_iter().flatten() {
            let instance: Instance =
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
            if !instance.workspace_only
                && !inventory
                    .instances
                    .iter()
                    .any(|current| current.name.eq_ignore_ascii_case(&instance.name))
            {
                inventory.instances.push(instance);
                if let Some(at) = previous["observed_at_unix_seconds"].as_u64() {
                    inventory.observed_at_unix_seconds = inventory.observed_at_unix_seconds.min(at);
                }
            }
        }
        for instance in &mut inventory.instances {
            if instance.workspace_only || instance.pending {
                continue;
            }
            instance.runtime.stale = true;
            for service in &mut instance.services {
                service.runtime.stale = true;
                service.summary = service.status_summary();
            }
            instance.summary = instance.status_summary();
        }
        Ok(inventory)
    }

    pub(crate) fn inspect_instances(&self, fresh: bool) -> Result<Vec<Instance>, String> {
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            let revision = self.inventory_revision()?;
            if let Some((_, observation)) =
                self.observation::<InventoryObservation>("instances", 0)?
                && !fresh
                && observation.revision == revision
                && state_stream::now().saturating_sub(observation.at) < 10
            {
                return observation.result;
            }
            if let Some(_lease) = self.observation_lease("instances")? {
                let revision = self.inventory_revision()?;
                if let Some((_, observation)) =
                    self.observation::<InventoryObservation>("instances", 0)?
                    && !fresh
                    && observation.revision == revision
                    && state_stream::now().saturating_sub(observation.at) < 10
                {
                    return observation.result;
                }
                // The revision precedes the read so concurrent mutations invalidate the cache.
                let result = crate::environments::docker::inspect(&self.config);
                self.save_observation(
                    "instances",
                    &InventoryObservation {
                        at: state_stream::now(),
                        revision,
                        result: result.clone(),
                    },
                )?;
                return result;
            }
            if Instant::now() >= deadline {
                return Err("shared instance observation timed out".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}
