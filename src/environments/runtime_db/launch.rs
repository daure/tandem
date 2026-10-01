use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Config, Kind};
use crate::{
    environments::compose,
    store::environments::{Manifest, Template, validate_instance_name, validate_name},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::environments) struct Snapshot {
    version: u8,
    pub name: String,
    pub template: String,
    pub directory: PathBuf,
    workspace: PathBuf,
    project: String,
    namespace: String,
    pub source: String,
    pub manifest: Option<Manifest>,
}

impl Snapshot {
    pub(in crate::environments) fn new(
        config: &Config,
        template: &Template,
        name: &str,
        source: String,
    ) -> Self {
        Self {
            version: 1,
            name: name.into(),
            template: template.name.clone(),
            directory: PathBuf::from(&template.directory),
            workspace: config.workspaces.join(name),
            project: config.project(name),
            namespace: config.namespace.clone(),
            source,
            manifest: Some(template.manifest.clone()),
        }
    }

    pub(super) fn legacy(
        config: &Config,
        template: &str,
        name: &str,
        source: String,
    ) -> Result<Self, String> {
        let snapshot = Self {
            version: 1,
            name: name.into(),
            template: template.into(),
            directory: config.templates.join(template),
            workspace: config.workspaces.join(name),
            project: config.project(name),
            namespace: config.namespace.clone(),
            source,
            manifest: None,
        };
        snapshot.validate(config, name)?;
        Ok(snapshot)
    }

    pub(in crate::environments) fn validate(
        &self,
        config: &Config,
        name: &str,
    ) -> Result<Value, String> {
        validate_instance_name(&self.name)?;
        validate_name(&self.template)?;
        if self.version != 1
            || !self.name.eq_ignore_ascii_case(name)
            || self.directory != config.templates.join(&self.template)
            || self.workspace != config.workspaces.join(&self.name)
            || self.project != config.project(&self.name)
            || self.namespace != config.namespace
            || self.source.len() > 262_144
        {
            return Err("launch snapshot ownership mismatch".into());
        }
        let model: Value = serde_json::from_str(&self.source).map_err(|error| error.to_string())?;
        if model["name"]
            .as_str()
            .is_some_and(|project| project != self.project)
        {
            return Err("launch snapshot project mismatch".into());
        }
        let services = model["services"]
            .as_object()
            .filter(|services| !services.is_empty())
            .ok_or("launch snapshot must declare services")?;
        for (service_name, service) in services {
            validate_name(service_name)?;
            for (key, expected) in [
                (compose::NAMESPACE, self.namespace.as_str()),
                (compose::KIND, "instance"),
                (compose::INSTANCE, self.name.as_str()),
                (compose::TEMPLATE, self.template.as_str()),
                (
                    compose::DIRECTORY,
                    self.directory.to_str().ok_or("invalid template path")?,
                ),
                (
                    compose::WORKSPACE,
                    self.workspace.to_str().ok_or("invalid workspace path")?,
                ),
            ] {
                if service["labels"][key]
                    .as_str()
                    .map(|value| value.replace("$$", "$"))
                    != Some(expected.into())
                {
                    return Err("launch snapshot ownership mismatch".into());
                }
            }
        }
        Ok(model)
    }

    pub(in crate::environments) fn encode(&self, config: &Config) -> Result<String, String> {
        self.validate(config, &self.name)?;
        let text = serde_json::to_string(self).map_err(|error| error.to_string())?;
        if text.len() > 1_048_576 {
            return Err("launch snapshot exceeds 1 MiB".into());
        }
        Ok(text)
    }
}

pub(in crate::environments) fn load(
    config: &Config,
    name: &str,
) -> Result<Option<Snapshot>, String> {
    super::load(config, name, Kind::Launch)?
        .map(|text| {
            let snapshot: Snapshot =
                serde_json::from_str(&text).map_err(|error| error.to_string())?;
            snapshot.validate(config, name)?;
            Ok(snapshot)
        })
        .transpose()
}

pub(in crate::environments) fn path(config: &Config, name: &str) -> Result<PathBuf, String> {
    validate_instance_name(name)?;
    let root = super::directory(config)?;
    let directory = root.join(name.to_ascii_lowercase());
    super::private_directory(&directory)?;
    Ok(directory.join("compose.json"))
}

pub(in crate::environments) fn materialize(
    config: &Config,
    name: &str,
) -> Result<Option<PathBuf>, String> {
    let Some(snapshot) = load(config, name)? else {
        return Ok(None);
    };
    let path = path(config, &snapshot.name)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err("rendered Compose must be a regular file".into());
        }
        Ok(_) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .map_err(|error| error.to_string())?;
            }
            if crate::environments::config::read_text(&path)? == snapshot.source {
                return Ok(Some(path));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("invalid runtime path")?)
            .map_err(|error| error.to_string())?;
    temporary
        .write_all(snapshot.source.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    temporary
        .persist(&path)
        .map_err(|error| error.to_string())?;
    Ok(Some(path))
}

pub(in crate::environments) fn remove(config: &Config, name: &str) -> Result<(), String> {
    let path = path(config, name)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err("rendered Compose must be a regular file".into());
        }
        Ok(_) => fs::remove_file(&path).map_err(|error| error.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    // A runtime directory may retain private migration backups or diagnostic files.
    if let Some(parent) = Path::new(&path).parent() {
        match fs::remove_dir(parent) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                ) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}
