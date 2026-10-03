use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use super::Providers;
use crate::{
    environments::{command, events::EventStore, gateway, lifecycle},
    store::{providers::Manifest, rules::Acceptance},
};

pub(crate) struct Deletion {
    manager: Providers,
    name: String,
    pub source: String,
    pub acceptances: Vec<Acceptance>,
    package: PathBuf,
    runtime: PathBuf,
    credential: PathBuf,
    _package_lock: gateway::Lock,
    _provider_lock: gateway::Lock,
    _source_lock: gateway::Lock,
}

impl Providers {
    pub(crate) fn begin_deletion(&self, name: &str) -> Result<Deletion, String> {
        crate::store::environments::validate_name(name)?;
        let package_lock = gateway::lock(&self.config, &format!("provider-package-{name}"))?;
        let provider_lock = gateway::lock(
            &self.config,
            &format!("provider-{}-{name}", self.config.namespace),
        )?;
        let package = self.config.home.join("templates/providers").join(name);
        let connection = self.database()?;
        let mut statement = connection
            .prepare("SELECT namespace, name, directory, manifest FROM provider_launches")
            .map_err(|error| error.to_string())?;
        let launches = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let saved = launches.iter().find(|(namespace, package_name, _, _)| {
            namespace == &self.config.namespace && package_name == name
        });
        if launches
            .iter()
            .any(|(namespace, package_name, directory, _)| {
                namespace != &self.config.namespace
                    && (package_name == name || Path::new(directory) == package)
            })
        {
            return Err("provider package is installed in another namespace; refusing shared package deletion".into());
        }
        let manifest: Manifest = if let Some((_, _, directory, manifest)) = saved {
            if Path::new(directory) != package {
                return Err("saved provider directory does not match its package path".into());
            }
            serde_json::from_str(manifest).map_err(|error| error.to_string())?
        } else {
            self.definition(name)?
                .manifest
                .ok_or("provider manifest missing")?
        };
        manifest.validate()?;
        let source_lock = gateway::lock(
            &self.config,
            &format!(
                "provider-source-{}-{}",
                self.config.namespace, manifest.name
            ),
        )?;
        for (namespace, package_name, _, saved_manifest) in &launches {
            if namespace == &self.config.namespace && package_name != name {
                let other: Manifest =
                    serde_json::from_str(saved_manifest).map_err(|error| error.to_string())?;
                if other.name == manifest.name {
                    return Err("provider identity is shared by another installation".into());
                }
            }
        }
        let runtime = self
            .config
            .home
            .join("runtime")
            .join(&self.config.namespace)
            .join("providers")
            .join(name);
        let credential = self
            .config
            .home
            .join("provider-credentials")
            .join(&self.config.namespace)
            .join(format!("{}.token", manifest.name));
        for path in [&package, &runtime, &credential] {
            validate_path(&self.config.home, path)?;
        }
        let store = EventStore::open(&self.config).map_err(|error| error.to_string())?;
        let acceptances = store
            .provider_deletion_acceptances(&manifest.name)
            .map_err(|error| error.to_string())?;
        // Keep launch identity until all external cleanup succeeds, including partial retries.
        if saved.is_none() {
            self.save_launch(&self.definition(name)?, "{}")?;
        }
        store
            .begin_provider_deletion(name, &manifest.name)
            .map_err(|error| error.to_string())?;
        Ok(Deletion {
            manager: self.clone(),
            name: name.into(),
            source: manifest.name,
            acceptances,
            package,
            runtime,
            credential,
            _package_lock: package_lock,
            _provider_lock: provider_lock,
            _source_lock: source_lock,
        })
    }

    pub(crate) fn delete_created_instance(
        &self,
        instance: &str,
        operation: &str,
        before_deletion: &crate::environments::BeforeDeletion<'_>,
    ) -> Result<(), String> {
        lifecycle::delete_for_provider(
            &self.config,
            instance,
            operation,
            std::sync::Arc::new(|_| {}),
            before_deletion,
        )
    }

    pub(super) fn ensure_not_deleting(&self, name: &str) -> Result<(), String> {
        let deleting: bool = self.database()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM provider_deletions WHERE namespace = ?1 AND name = ?2)",
            rusqlite::params![self.config.namespace, name], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if deleting {
            return Err("provider deletion is incomplete; retry delete_provider".into());
        }
        Ok(())
    }
}

impl Deletion {
    pub(crate) fn remove_collector(&self) -> Result<(), String> {
        let manager = &self.manager;
        let project = manager.project(&self.name);
        let mut list = command::docker();
        list.args(["volume", "ls", "--format", "{{.Name}}"]);
        let names = command::run(list, Duration::from_secs(10), None)?;
        let volume = format!("{project}_state");
        let has_volume = names.lines().any(|name| name == volume);
        if has_volume {
            let mut inspect = command::docker();
            inspect.args(["volume", "inspect", &volume]);
            let models: Vec<serde_json::Value> =
                serde_json::from_str(&command::run(inspect, Duration::from_secs(10), None)?)
                    .map_err(|error| error.to_string())?;
            if models.len() != 1
                || models[0]["Labels"]["io.tandem.provider-namespace"] != manager.config.namespace
                || models[0]["Labels"]["io.tandem.provider-name"] != self.name
            {
                return Err("provider checkpoint volume has unverifiable ownership".into());
            }
        }
        if let Some(container) = manager.container(&self.name)? {
            let id = container["Id"].as_str().ok_or("collector ID missing")?;
            if container["State"]["Paused"].as_bool() == Some(true) {
                let mut unpause = command::docker();
                unpause.args(["unpause", id]);
                command::run(unpause, Duration::from_secs(20), None)?;
            }
            if container["State"]["Running"].as_bool() == Some(true) {
                let mut stop = command::docker();
                stop.args(["stop", "--timeout", "10", id]);
                command::run(stop, Duration::from_secs(30), None)?;
            }
            manager.inspect_containers(&self.name, &[id])?;
            let mut remove = command::docker();
            remove.args(["rm", "--volumes", id]);
            command::run(remove, Duration::from_secs(30), None)?;
        }
        if manager.container(&self.name)?.is_some() {
            return Err("provider collector still exists; retry deletion".into());
        }
        if has_volume {
            let mut remove = command::docker();
            remove.args(["volume", "rm", &volume]);
            command::run(remove, Duration::from_secs(30), None)?;
            let mut list = command::docker();
            list.args(["volume", "ls", "--format", "{{.Name}}"]);
            if command::run(list, Duration::from_secs(10), None)?
                .lines()
                .any(|name| name == volume)
            {
                return Err("provider checkpoint volume still exists; retry deletion".into());
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<i64, String> {
        for path in [&self.credential, &self.runtime, &self.package] {
            validate_path(&self.manager.config.home, path)?;
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_dir() => {
                    fs::remove_dir_all(path).map_err(|error| error.to_string())?
                }
                Ok(_) => fs::remove_file(path).map_err(|error| error.to_string())?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        EventStore::open(&self.manager.config)
            .and_then(|store| store.delete_provider(&self.name, &self.source))
            .map_err(|error| error.to_string())
    }
}

fn validate_path(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "provider deletion path escapes Tandem home")?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("unsafe provider deletion path".into());
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("provider deletion path contains a symlink".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}
