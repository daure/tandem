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
    store::opencode::{Snapshot, observation::Failure},
};

pub(super) async fn run(
    mut observer: Observer,
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
        let Some(owner) = integration.upgrade() else {
            return;
        };
        let deadline = owner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retention
            .next_retry();
        drop(owner);
        let flags = tokio::select! {
            flags = signal.wait() => flags,
            _ = async {
                if let Some(deadline) = deadline {
                    tokio::time::sleep_until(deadline.into()).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => REMOTE,
            _ = samples.tick() => {
                let Some(owner) = integration.upgrade() else { return; };
                if !settings.opencode_enabled() { return; }
                let mut snapshot = owner.state.lock().unwrap_or_else(|error| error.into_inner()).snapshot.clone();
                drop(owner);
                let result = observer.sample_resources(snapshot.resources.clone()).await;
                observer.refresh_tab_order(&mut snapshot).await;
                let Some(owner) = integration.upgrade() else { return; };
                let mut state = owner.state.lock().unwrap_or_else(|error| error.into_inner());
                if state.generation != generation || !settings.opencode_enabled() { return; }
                state.snapshot.zellij_tabs = snapshot.zellij_tabs;
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
            observer.excluded = state.retention.exclusions();
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
        let rediscover = changes.evidence_changed();
        if rediscover {
            let Some(owner) = integration.upgrade() else {
                return;
            };
            owner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retention
                .rediscover();
            observer.excluded = Default::default();
            if let Err(error) = changes.sync(&observer, &previous).await {
                publish(&integration, generation, Err(error));
                return;
            }
        }
        if flags & REMOTE == 0 && !local {
            continue;
        }
        let mut result = tokio::time::timeout(
            Duration::from_secs(15),
            observer.observe_changes(&roots, previous, flags & REMOTE != 0 || rediscover),
        )
        .await
        .unwrap_or_else(|_| Err("OpenCode observation timed out".into()));
        if let Ok(snapshot) = &mut result {
            let errors = changes.errors();
            for session in &mut snapshot.sessions {
                session.stale |= errors.contains_key(&session.server);
            }
            for client in &mut snapshot.clients {
                client.stale |= errors.contains_key(&client.server);
            }
            for (server, error) in errors {
                snapshot
                    .observation
                    .failures
                    .push(Failure::server(&server, error));
            }
            snapshot.error = snapshot.observation_error();
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
            state.snapshot.observation.verified_panes.clear();
            for resource in &mut state.snapshot.resources {
                resource.mark_stale(error.clone());
            }
            for session in &mut state.snapshot.sessions {
                session.stale = true;
            }
            for client in &mut state.snapshot.clients {
                client.stale = true;
            }
            state.snapshot.observation.failures = vec![Failure::global(error.clone())];
            state.snapshot.error = Some(error);
        }
    }
    let State {
        snapshot,
        retention,
        ..
    } = &mut *state;
    let abandoned = retention.observe(snapshot);
    retention.discard_abandoned(snapshot);
    drop(state);
    if !abandoned.is_empty() {
        tokio::task::spawn_blocking(move || {
            for message in abandoned {
                crate::diagnostics::record_error(
                    "OpenCode observation",
                    &std::io::Error::other(message),
                );
            }
        });
    }
}
