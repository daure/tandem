use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use super::{Integration, State};
use crate::{
    environments::opencode::{
        Observer,
        events::{Changes, LOCAL, REMOTE, Signal},
    },
    store::opencode::Snapshot,
};

pub(super) async fn run(
    observer: Observer,
    integration: Weak<Integration>,
    settings: Arc<crate::service::settings::Settings>,
    generation: u64,
    signal: Arc<Signal>,
) {
    let watched = observer.clone();
    let notifications = Arc::clone(&signal);
    let mut changes =
        match tokio::task::spawn_blocking(move || Changes::new(&watched, notifications))
            .await
            .unwrap_or_else(|error| Err(error.to_string()))
        {
            Ok(changes) => changes,
            Err(error) => {
                publish(&integration, generation, Err(error));
                return;
            }
        };
    let mut samples = tokio::time::interval(Duration::from_secs(5));
    samples.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    signal.send(REMOTE);
    loop {
        let flags = tokio::select! {
            flags = signal.wait() => flags,
            _ = samples.tick() => {
                let Some(owner) = integration.upgrade() else { return; };
                if !settings.opencode_enabled() { return; }
                let previous = owner.state.lock().unwrap_or_else(|error| error.into_inner()).snapshot.resources.clone();
                drop(owner);
                let result = observer.sample_resources(previous).await;
                let Some(owner) = integration.upgrade() else { return; };
                let mut state = owner.state.lock().unwrap_or_else(|error| error.into_inner());
                if state.generation != generation || !settings.opencode_enabled() { return; }
                match result {
                    Ok(resources) => state.snapshot.resources = resources,
                    Err(error) => for resource in &mut state.snapshot.resources { resource.mark_stale(error.clone()); },
                }
                // Sampling also detects crashed clients and expired presence leases.
                LOCAL
            }
        };
        if flags & REMOTE != 0 {
            tokio::time::sleep(Duration::from_millis(75)).await;
        }
        if !settings.opencode_enabled() {
            return;
        }
        let Some(owner) = integration.upgrade() else {
            return;
        };
        let (roots, previous) = {
            let state = owner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.generation != generation {
                return;
            }
            (state.roots.clone(), state.snapshot.clone())
        };
        drop(owner);
        let local = match changes.sync(&observer, &previous).await {
            Ok(changed) => changed,
            Err(error) => {
                publish(&integration, generation, Err(error));
                return;
            }
        };
        if flags & REMOTE == 0 && !local {
            continue;
        }
        let mut result = tokio::time::timeout(
            Duration::from_secs(15),
            observer.observe_changes(&roots, previous, flags & REMOTE != 0),
        )
        .await
        .unwrap_or_else(|_| Err("OpenCode observation timed out".into()));
        if let Ok(snapshot) = &mut result {
            let errors = signal.errors();
            for session in &mut snapshot.sessions {
                session.stale |= errors.contains_key(&session.server);
            }
            for client in &mut snapshot.clients {
                client.stale |= errors.contains_key(&client.server);
            }
            if !errors.is_empty() {
                snapshot.error = Some(errors.values().cloned().collect::<Vec<_>>().join("\n"));
            }
        }
        if !settings.opencode_enabled() {
            return;
        }
        publish(&integration, generation, result);
    }
}

#[cfg(test)]
#[path = "tests/observation.rs"]
mod tests;

fn publish(integration: &Weak<Integration>, generation: u64, result: Result<Snapshot, String>) {
    let Some(owner) = integration.upgrade() else {
        return;
    };
    let mut state = owner
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if state.generation != generation {
        return;
    }
    match result {
        Ok(snapshot) => state.snapshot = snapshot,
        Err(error) => {
            for resource in &mut state.snapshot.resources {
                resource.mark_stale(error.clone());
            }
            for session in &mut state.snapshot.sessions {
                session.stale = true;
            }
            for client in &mut state.snapshot.clients {
                client.stale = true;
            }
            state.snapshot.error = Some(error);
        }
    }
    let State {
        snapshot,
        retention,
        ..
    } = &mut *state;
    retention.observe(snapshot);
}
