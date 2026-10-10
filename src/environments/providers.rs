use std::{collections::BTreeMap, fs, io::Write, time::Duration};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::{
    command,
    config::{Config, read_text},
    events::EventStore,
    gateway, provider_sidecar,
};
use crate::store::providers::{
    Action, ActionError, ActionOutcome, Manifest, Provider, RuntimeObservation, Snapshot, Status,
};

mod deletion;
mod streams;

#[derive(Clone)]
pub(crate) struct Providers {
    config: Config,
}

impl Providers {
    pub(crate) fn new(config: &Config) -> Result<Self, String> {
        let manager = Self {
            config: config.clone(),
        };
        manager
            .database()?
            .execute_batch(include_str!("../../migrations/0007_providers.sql"))
            .map_err(|error| error.to_string())?;
        Ok(manager)
    }

    fn database(&self) -> Result<Connection, String> {
        let connection = Connection::open(self.config.home.join("settings.sqlite3"))
            .map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        Ok(connection)
    }

    fn definition(&self, name: &str) -> Result<Provider, String> {
        crate::store::environments::validate_name(name)?;
        let root = self.config.home.join("templates/providers");
        let directory = root.join(name);
        if fs::canonicalize(&directory).map_err(|error| error.to_string())? != directory
            || !directory.join("Dockerfile").is_file()
        {
            return Err("provider requires a real package directory and Dockerfile".into());
        }
        let manifest: Manifest =
            serde_json::from_str(&read_text(&directory.join("provider.json"))?)
                .map_err(|error| error.to_string())?;
        manifest.validate()?;
        Ok(Provider {
            name: name.into(),
            directory: directory.display().to_string(),
            manifest: Some(manifest),
            available: true,
            status: Status::NotStarted,
            container_id: None,
            error: None,
            operation: None,
            streams: Vec::new(),
        })
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot, String> {
        let root = self.config.home.join("templates/providers");
        let mut providers = BTreeMap::new();
        if root.exists() {
            for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                if !entry.path().join("provider.json").exists() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let provider = self.definition(&name).unwrap_or_else(|error| Provider {
                    name: name.clone(),
                    directory: entry.path().display().to_string(),
                    manifest: None,
                    available: false,
                    status: Status::Unknown,
                    container_id: None,
                    error: Some(error),
                    operation: None,
                    streams: Vec::new(),
                });
                providers.insert(name, provider);
            }
        }
        let connection = self.database()?;
        let mut statement = connection.prepare("SELECT name, directory, manifest, error FROM provider_launches WHERE namespace = ?1").map_err(|error| error.to_string())?;
        for row in statement
            .query_map([&self.config.namespace], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?
        {
            let (name, directory, manifest, error) = row.map_err(|error| error.to_string())?;
            let provider = providers.entry(name.clone()).or_insert(Provider {
                name,
                directory,
                manifest: serde_json::from_str(&manifest).ok(),
                available: false,
                status: Status::Stopped,
                container_id: None,
                error: None,
                operation: None,
                streams: Vec::new(),
            });
            provider.error = provider.error.clone().or(error);
            provider.manifest = serde_json::from_str(&manifest)
                .ok()
                .or(provider.manifest.clone());
        }
        if providers.is_empty() {
            return Ok(Snapshot::default());
        }
        let inventory = self.container_inventory(&providers);
        let mut error = None;
        for provider in providers.values_mut() {
            let container = match &inventory {
                Ok(projects) => {
                    let ids = &projects[&self.project(&provider.name)];
                    if ids.is_empty() {
                        Ok(None)
                    } else {
                        let ids: Vec<_> = ids.iter().map(String::as_str).collect();
                        self.inspect_containers(&provider.name, &ids).map(Some)
                    }
                }
                Err(issue) => Err(issue.clone()),
            };
            match container {
                Ok(Some(container)) => {
                    provider.container_id = container["Id"].as_str().map(str::to_owned);
                    provider.status = container_status(Some(&container));
                }
                Ok(None) => {}
                Err(issue) => {
                    provider.status = Status::Unknown;
                    provider.error = Some(issue.clone());
                    error = Some(issue);
                }
            }
            if let Some(manifest) = &provider.manifest {
                provider.streams = EventStore::open(&self.config)
                    .and_then(|store| store.provider_streams(manifest, provider.status))
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(Snapshot {
            providers: providers.into_values().collect(),
            error,
        })
    }

    fn project(&self, name: &str) -> String {
        format!("{}-provider-{name}", self.config.namespace)
    }

    fn container_inventory(
        &self,
        providers: &BTreeMap<String, Provider>,
    ) -> Result<BTreeMap<String, Vec<String>>, String> {
        let mut projects: BTreeMap<_, Vec<String>> = providers
            .keys()
            .map(|name| (self.project(name), Vec::new()))
            .collect();
        let mut list = command::docker();
        list.args([
            "ps",
            "--all",
            "--filter",
            "label=com.docker.compose.project",
            "--format",
            "{{.ID}}\t{{.Label \"com.docker.compose.project\"}}",
        ]);
        let output = command::run(list, Duration::from_secs(10), None)?;
        for line in output.lines().filter(|line| !line.is_empty()) {
            let (id, project) = line
                .split_once('\t')
                .filter(|(id, _)| !id.is_empty())
                .ok_or("Docker provider inventory row is invalid")?;
            if let Some(ids) = projects.get_mut(project) {
                ids.push(id.to_owned());
            }
        }
        Ok(projects)
    }

    fn container(&self, name: &str) -> Result<Option<Value>, String> {
        let project = self.project(name);
        let mut list = command::docker();
        list.args([
            "ps",
            "--all",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--format",
            "{{.ID}}",
        ]);
        let output = command::run(list, Duration::from_secs(10), None)?;
        let ids: Vec<_> = output.split_whitespace().collect();
        if ids.is_empty() {
            return Ok(None);
        }
        self.inspect_containers(name, &ids).map(Some)
    }

    fn inspect_containers(&self, name: &str, ids: &[&str]) -> Result<Value, String> {
        let mut inspect = command::docker();
        inspect.arg("inspect").args(ids);
        let output = command::run(inspect, Duration::from_secs(10), None)?;
        let containers: Vec<Value> =
            serde_json::from_str(&output).map_err(|error| error.to_string())?;
        if containers.len() != 1 {
            return Err("provider project must contain exactly one owned collector".into());
        }
        let container = containers
            .into_iter()
            .next()
            .ok_or("collector disappeared")?;
        validate_owner(
            &container,
            &self.config.namespace,
            name,
            &self.project(name),
        )?;
        Ok(container)
    }

    pub(crate) fn action(&self, name: &str, action: Action) -> Result<ActionOutcome, ActionError> {
        self.run_action(name, action, || self.execute(name, action))
    }

    fn run_action<T>(
        &self,
        name: &str,
        action: Action,
        execute: impl FnOnce() -> Result<T, ActionError>,
    ) -> Result<T, ActionError> {
        crate::store::environments::validate_name(name)?;
        let _package = gateway::shared_lock(&self.config, &format!("provider-package-{name}"))?;
        let _gate = gateway::lock(
            &self.config,
            &format!("provider-{}-{name}", self.config.namespace),
        )?;
        self.ensure_not_deleting(name)?;
        let ingestion = if action == Action::Logs {
            None
        } else {
            self.set_ingestion_for_action(name, action)?
        };
        let result = execute();
        if let Some((store, source, previous)) = ingestion
            && result.is_err()
            && (action == Action::Start || matches!(result, Err(ActionError::Unavailable(_))))
        {
            store
                .set_ingestion_enabled(&source, previous)
                .map_err(|error| error.to_string())?;
        }
        if action != Action::Logs && !matches!(result, Err(ActionError::Unavailable(_))) {
            self.database()?
                .execute(
                    "UPDATE provider_launches SET error = ?3 WHERE namespace = ?1 AND name = ?2",
                    params![
                        self.config.namespace,
                        name,
                        result.as_ref().err().map(ToString::to_string)
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        result
    }

    fn set_ingestion_for_action(
        &self,
        name: &str,
        action: Action,
    ) -> Result<Option<(EventStore, String, bool)>, String> {
        let manifest: Option<String> = self
            .database()?
            .query_row(
                "SELECT manifest FROM provider_launches WHERE namespace = ?1 AND name = ?2",
                params![self.config.namespace, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(manifest) = manifest else {
            return Ok(None);
        };
        let manifest: Manifest =
            serde_json::from_str(&manifest).map_err(|error| error.to_string())?;
        let store = EventStore::open(&self.config).map_err(|error| error.to_string())?;
        // Persist before Docker work so the detached sidecar observes Stop immediately.
        let previous = store
            .set_ingestion_enabled(&manifest.name, action != Action::Stop)
            .map_err(|error| error.to_string())?;
        Ok(Some((store, manifest.name, previous)))
    }

    fn execute(&self, name: &str, action: Action) -> Result<ActionOutcome, ActionError> {
        let container = self.container(name)?;
        if let Some(reason) =
            action.unavailable_reason(container_status(container.as_ref()), container.is_some())
        {
            return Err(ActionError::Unavailable(reason));
        }
        if action == Action::Start {
            return self
                .start_collector(name, container.as_ref(), None)
                .map_err(Into::into);
        }
        let container = container.ok_or("provider has no owned container")?;
        let id = container["Id"].as_str().ok_or("collector ID missing")?;
        let paused = container["State"]["Paused"].as_bool() == Some(true);
        if action == Action::Stop {
            self.invalidate_stream_controls(name)?;
        }
        if action == Action::Stop && paused {
            let mut unpause = command::docker();
            unpause.args(["unpause", id]);
            command::run(unpause, Duration::from_secs(20), None)?;
        }
        let mut command_ = command::docker();
        match action {
            Action::Stop => {
                command_.args(["stop", "--timeout", "10", id]);
            }
            Action::Logs => {
                command_.args(["logs", "--tail", "100", id]);
            }
            Action::Start => unreachable!(),
        }
        let output = if action == Action::Logs {
            command::run_combined(command_, Duration::from_secs(30))?
        } else {
            command::run(command_, Duration::from_secs(30), None)?
        };
        if action == Action::Logs {
            return Ok(ActionOutcome {
                message: output,
                runtime: None,
            });
        }
        let observed = self.inspect_containers(name, &[id])?;
        let running = observed["State"]["Running"].as_bool() == Some(true);
        if running {
            return Err("provider did not reach the requested runtime state".into());
        }
        observed_outcome(format!("Provider {name}: {action:?}"), &observed).map_err(Into::into)
    }

    fn start_collector(
        &self,
        name: &str,
        container: Option<&Value>,
        only_stream: Option<&str>,
    ) -> Result<ActionOutcome, String> {
        if let Some(container) = container
            && container["State"]["Running"].as_bool() == Some(true)
        {
            provider_sidecar::ensure(&self.config)?;
            self.prepare_stream_controls(name, only_stream)?;
            let id = container["Id"].as_str().ok_or("collector ID missing")?;
            if container["State"]["Paused"].as_bool() == Some(true) {
                let mut unpause = command::docker();
                unpause.args(["unpause", id]);
                command::run(unpause, Duration::from_secs(20), None)?;
            }
            let observed = self.inspect_containers(name, &[id])?;
            if container_status(Some(&observed)) != Status::Running {
                return Err("provider collector failed to run; inspect its logs".into());
            }
            return observed_outcome(format!("Provider {name} started"), &observed);
        }
        self.start(name, only_stream)
    }

    fn start(&self, name: &str, only_stream: Option<&str>) -> Result<ActionOutcome, String> {
        let definition = self.definition(name)?;
        let manifest = definition
            .manifest
            .as_ref()
            .ok_or("provider manifest missing")?;
        if let Some(stream) = only_stream
            && (!manifest.stream_control
                || !manifest
                    .streams
                    .iter()
                    .any(|declaration| declaration.name == stream))
        {
            return Err("Provider does not support controls for this stream".into());
        }
        let _source = gateway::lock(
            &self.config,
            &format!(
                "provider-source-{}-{}",
                self.config.namespace, manifest.name
            ),
        )?;
        let token = EventStore::open(&self.config)
            .map_err(|error| error.to_string())?
            .provider_credential_file(&manifest.name)
            .map_err(|error| error.to_string())?;
        let origin = provider_sidecar::ensure(&self.config)?;
        let project = self.project(name);
        let labels = json!({"io.tandem.provider-namespace": self.config.namespace, "io.tandem.provider-name": name});
        let compose = json!({"services": {"collector": {
            "build": {"context": definition.directory}, "network_mode": "host", "restart": "unless-stopped",
            "labels": labels, "environment": {"TANDEM_EVENTS_URL": origin, "TANDEM_PROVIDER_TOKEN_FILE": "/run/secrets/provider-token"},
            "volumes": [{"type": "volume", "source": "state", "target": "/state"},
                {"type": "bind", "source": token, "target": "/run/secrets/provider-token", "read_only": true, "bind": {"create_host_path": false}}]
        }}, "volumes": {"state": {"labels": labels}}});
        let mut list = command::docker();
        list.args(["volume", "ls", "--format", "{{.Name}}"]);
        let volumes = command::run(list, Duration::from_secs(10), None)?;
        let volume = format!("{project}_state");
        if volumes.lines().any(|name| name == volume) {
            let mut inspect = command::docker();
            inspect.args(["volume", "inspect", &volume]);
            let output = command::run(inspect, Duration::from_secs(10), None)?;
            let models: Vec<Value> =
                serde_json::from_str(&output).map_err(|error| error.to_string())?;
            let owned = models.first().is_some_and(|model| {
                model["Labels"]["io.tandem.provider-namespace"] == self.config.namespace
                    && model["Labels"]["io.tandem.provider-name"] == name
            });
            if !owned {
                return Err("provider checkpoint volume has unverifiable ownership".into());
            }
        }
        let source = serde_json::to_string(&compose).map_err(|error| error.to_string())?;
        self.save_launch(&definition, &source)?;
        EventStore::open(&self.config)
            .and_then(|store| store.prepare_streams_for(manifest, only_stream))
            .map_err(|error| error.to_string())?;
        let runtime = super::runtime_db::directory(&self.config)?
            .join("providers")
            .join(name);
        fs::create_dir_all(&runtime).map_err(|error| error.to_string())?;
        if fs::canonicalize(&runtime).map_err(|error| error.to_string())? != runtime {
            return Err("unsafe provider runtime directory".into());
        }
        let mut file =
            tempfile::NamedTempFile::new_in(&runtime).map_err(|error| error.to_string())?;
        file.write_all(source.as_bytes())
            .and_then(|()| file.as_file().sync_all())
            .map_err(|error| error.to_string())?;
        let compose_path = runtime.join("compose.json");
        file.persist(&compose_path)
            .map_err(|error| error.to_string())?;
        let mut up = command::docker();
        up.args(["compose", "-p", &project, "-f"])
            .arg(compose_path)
            .args(["up", "-d", "--build", "--no-deps", "collector"]);
        command::run(up, Duration::from_secs(180), None)?;
        let observed = self
            .container(name)?
            .ok_or("provider startup produced no owned collector")?;
        if observed["State"]["Running"].as_bool() != Some(true)
            || observed["State"]["Paused"].as_bool() == Some(true)
        {
            return Err("provider collector failed to run; inspect its logs".into());
        }
        EventStore::open(&self.config)
            .and_then(|store| store.collector_started(&manifest.name))
            .map_err(|error| error.to_string())?;
        observed_outcome(format!("Provider {name} started"), &observed)
    }

    fn save_launch(&self, definition: &Provider, source: &str) -> Result<(), String> {
        let manifest = definition
            .manifest
            .as_ref()
            .ok_or("provider manifest missing")?;
        let mut connection = self.database()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        {
            let mut statement = transaction
                .prepare("SELECT name, manifest FROM provider_launches WHERE namespace = ?1")
                .map_err(|error| error.to_string())?;
            for row in statement
                .query_map([&self.config.namespace], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| error.to_string())?
            {
                let (name, saved) = row.map_err(|error| error.to_string())?;
                let saved: Manifest =
                    serde_json::from_str(&saved).map_err(|error| error.to_string())?;
                if name != definition.name && saved.name == manifest.name {
                    return Err("provider identity already belongs to another package".into());
                }
                if name == definition.name && saved.name != manifest.name {
                    return Err(
                        "provider identity is fixed for an installed package; use a new package"
                            .into(),
                    );
                }
            }
        }
        transaction.execute("INSERT INTO provider_launches(namespace, name, directory, manifest, compose) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(namespace, name) DO UPDATE SET directory = excluded.directory, manifest = excluded.manifest, compose = excluded.compose, error = NULL",
            params![self.config.namespace, definition.name, definition.directory, serde_json::to_string(manifest).map_err(|error| error.to_string())?, source])
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }
}

fn observed_outcome(message: String, container: &Value) -> Result<ActionOutcome, String> {
    Ok(ActionOutcome {
        message,
        runtime: Some(RuntimeObservation {
            status: container_status(Some(container)),
            container_id: container["Id"]
                .as_str()
                .ok_or("collector ID missing")?
                .into(),
        }),
    })
}

fn container_status(container: Option<&Value>) -> Status {
    match container {
        Some(container) if container["State"]["Paused"].as_bool() == Some(true) => Status::Paused,
        Some(container) if container["State"]["Running"].as_bool() == Some(true) => Status::Running,
        Some(_) => Status::Stopped,
        None => Status::NotStarted,
    }
}

fn validate_owner(
    container: &Value,
    namespace: &str,
    name: &str,
    project: &str,
) -> Result<(), String> {
    let labels = &container["Config"]["Labels"];
    if labels["io.tandem.provider-namespace"] != namespace
        || labels["io.tandem.provider-name"] != name
        || labels["com.docker.compose.project"] != project
        || labels["com.docker.compose.service"] != "collector"
    {
        return Err("provider container has unverifiable ownership".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/providers.rs"]
mod tests;
