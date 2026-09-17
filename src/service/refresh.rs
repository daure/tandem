use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension};
use tokio::sync::oneshot;

use super::settings::Settings;
use crate::environments::Environments;

const CHANGE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS refresh_revisions (
    scope TEXT PRIMARY KEY, revision INTEGER NOT NULL
);";

#[derive(Clone, Copy)]
pub(super) enum Refresh {
    Templates,
    Instances,
    Settings,
    Inventory,
    All,
}

impl Refresh {
    fn targets(self) -> [bool; 3] {
        match self {
            Self::Templates => [true, false, false],
            Self::Instances => [false, true, false],
            Self::Settings => [false, false, true],
            Self::Inventory => [true, true, false],
            Self::All => [true; 3],
        }
    }

    pub(super) fn for_operation(action: &str) -> Self {
        match action {
            "create_template" => Self::Templates,
            "remove_template" => Self::Inventory,
            _ => Self::Instances,
        }
    }
}

#[derive(Clone)]
pub(super) struct RefreshNotifier {
    database: PathBuf,
    scopes: [String; 3],
}

impl RefreshNotifier {
    pub(super) fn publish(&self, refresh: Refresh) {
        if let Err(error) = self.publish_result(refresh) {
            crate::diagnostics::record_error("cannot publish inventory change", &error);
        }
    }

    fn publish_result(&self, refresh: Refresh) -> rusqlite::Result<()> {
        let mut connection = Connection::open(&self.database)?;
        let transaction = connection.transaction()?;
        for (scope, changed) in self.scopes.iter().zip(refresh.targets()) {
            if changed {
                transaction.execute(
                    "INSERT INTO refresh_revisions(scope, revision) VALUES (?1, 1)
                     ON CONFLICT(scope) DO UPDATE SET revision = revision + 1",
                    [scope],
                )?;
            }
        }
        transaction.commit()
    }
}

pub(super) struct RefreshWorker {
    requests: mpsc::Sender<RefreshRequest>,
    pub(super) notifier: RefreshNotifier,
}

struct RefreshRequest {
    refresh: Refresh,
    completion: Option<oneshot::Sender<Result<(), String>>>,
}

impl From<Refresh> for RefreshRequest {
    fn from(refresh: Refresh) -> Self {
        Self {
            refresh,
            completion: None,
        }
    }
}

impl RefreshWorker {
    pub(super) fn start(
        environments: Arc<Environments>,
        settings: Arc<Settings>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let notifier = RefreshNotifier {
            database: environments.config.home.join("settings.sqlite3"),
            scopes: [
                "templates".into(),
                format!("instances:{}", environments.config.namespace),
                "settings".into(),
            ],
        };
        let connection = Connection::open(&notifier.database)?;
        connection.execute_batch(SCHEMA)?;
        let scopes = notifier.scopes.clone();
        let (requests, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("tandem-refresh".into())
            .spawn(move || {
                run(connection, scopes, receiver, |targets, manual| {
                    let mut errors = Vec::new();
                    if targets[0] {
                        environments.refresh_templates();
                    }
                    if targets[2]
                        && let Err(error) = settings.refresh_open_command()
                    {
                        errors.push(format!("Settings: {error}"));
                    }
                    if targets[1] {
                        environments.refresh_instances();
                        if let Some(request) = environments.begin_resource_sample(manual) {
                            environments.sample_resources(request);
                        }
                    }
                    let snapshot = environments.snapshot();
                    errors.extend(snapshot.error);
                    if targets[0] {
                        errors.extend(snapshot.templates.iter().filter_map(|template| {
                            template
                                .error
                                .as_ref()
                                .map(|error| format!("{}: {error}", template.name))
                        }));
                    }
                    if targets[1] {
                        errors.extend(snapshot.resource_error);
                    }
                    if errors.is_empty() {
                        Ok(())
                    } else {
                        Err(errors.join("\n"))
                    }
                });
            })?;
        Ok(Self { requests, notifier })
    }

    pub(super) fn request(&self, refresh: Refresh) {
        if let Err(error) = self.requests.send(refresh.into()) {
            crate::diagnostics::record_error("inventory refresh worker stopped", &error);
        }
    }

    pub(super) fn request_completion(
        &self,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        let (sender, receiver) = oneshot::channel();
        self.requests
            .send(RefreshRequest {
                refresh: Refresh::All,
                completion: Some(sender),
            })
            .map_err(|_| "Inventory refresh worker stopped".to_owned())?;
        Ok(receiver)
    }
}

fn revisions(connection: &Connection, scopes: &[String; 3]) -> rusqlite::Result<[i64; 3]> {
    let mut versions = [0; 3];
    let mut statement =
        connection.prepare_cached("SELECT revision FROM refresh_revisions WHERE scope = ?1")?;
    for (index, scope) in scopes.iter().enumerate() {
        versions[index] = statement
            .query_row([scope], |row| row.get(0))
            .optional()?
            .unwrap_or(0);
    }
    Ok(versions)
}

fn run(
    connection: Connection,
    scopes: [String; 3],
    receiver: mpsc::Receiver<RefreshRequest>,
    mut refresh: impl FnMut([bool; 3], bool) -> Result<(), String>,
) {
    // MCP-only processes publish changes without running a background inventory observer.
    let Ok(first) = receiver.recv() else { return };
    let mut targets = first.refresh.targets();
    let mut completions: Vec<_> = first.completion.into_iter().collect();
    let mut seen = [0; 3];
    loop {
        for request in receiver.try_iter() {
            for (target, requested) in targets.iter_mut().zip(request.refresh.targets()) {
                *target |= requested;
            }
            completions.extend(request.completion);
        }
        let mut errors = Vec::new();
        match revisions(&connection, &scopes) {
            Ok(current) => {
                for index in 0..3 {
                    targets[index] |= current[index] != seen[index];
                }
                // Read before refreshing so a concurrent mutation is picked up next time.
                seen = current;
            }
            Err(error) => {
                crate::diagnostics::record_error("cannot read inventory changes", &error);
                errors.push(format!("Cannot read inventory changes: {error}"));
            }
        }
        if targets.iter().any(|target| *target)
            && let Err(error) = refresh(targets, !completions.is_empty())
        {
            errors.push(error);
        }
        let result = if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        };
        for completion in completions.drain(..) {
            let _ = completion.send(result.clone());
        }
        targets = match receiver.recv_timeout(CHANGE_CHECK_INTERVAL) {
            Ok(request) => {
                completions.extend(request.completion);
                request.refresh.targets()
            }
            Err(mpsc::RecvTimeoutError::Timeout) => [false; 3],
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
    }
}

#[cfg(test)]
#[path = "tests/refresh.rs"]
mod tests;
