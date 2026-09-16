use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    compose,
    config::{Config, private_file, read_text},
};
use crate::store::environments::{validate_instance_name, validate_name};

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Owner {
    namespace: String,
    template: String,
    directory: PathBuf,
    instance: String,
}

fn record_path(config: &Config, directory: &Path, name: &str) -> PathBuf {
    directory.join(format!(".tandem-{}-{name}.owner.json", config.namespace))
}

pub(super) fn record(
    config: &Config,
    template: &str,
    directory: &Path,
    name: &str,
) -> Result<(), String> {
    validate_name(template)?;
    validate_instance_name(name)?;
    let owner = Owner {
        namespace: config.namespace.clone(),
        template: template.into(),
        directory: directory.into(),
        instance: name.into(),
    };
    let path = record_path(config, directory, name);
    if path.try_exists().map_err(|error| error.to_string())? {
        let existing: Owner =
            serde_json::from_str(&read_record(&path)?).map_err(|error| error.to_string())?;
        if existing != owner {
            return Err(format!("conflicting instance ownership: {name}"));
        }
        return Ok(());
    }
    let text = serde_json::to_vec(&owner).map_err(|error| error.to_string())?;
    private_file(&path, true)
        .and_then(|mut file| file.write_all(&text))
        .map_err(|error| error.to_string())
}

pub(super) fn forget(config: &Config, template: &str, name: &str) -> Result<(), String> {
    let path = record_path(config, &config.templates.join(template), name);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn instances(
    config: &Config,
    template: &str,
    directory: &Path,
) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    let prefix = format!(".tandem-{}-", config.namespace);
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let filename = entry.file_name().to_string_lossy().into_owned();
        let Some(suffix) = filename.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(name) = suffix.strip_suffix(".owner.json") {
            validate_instance_name(name)?;
            let owner: Owner = serde_json::from_str(&read_record(&entry.path())?)
                .map_err(|error| error.to_string())?;
            if owner.namespace != config.namespace
                || owner.template != template
                || owner.directory != directory
                || owner.instance != name
            {
                return Err(format!("conflicting ownership record: {filename}"));
            }
            names.insert(name.into());
        } else if let Some(name) = suffix.strip_suffix(".compose.json") {
            validate_instance_name(name)?;
            let model: Value = serde_json::from_str(&read_record(&entry.path())?)
                .map_err(|error| error.to_string())?;
            let services = model["services"]
                .as_object()
                .filter(|services| !services.is_empty())
                .ok_or("rendered Compose has no services")?;
            let workspace = config.workspaces.join(name);
            for service in services.values() {
                let labels = &service["labels"];
                for (key, expected) in [
                    (compose::NAMESPACE, config.namespace.as_str()),
                    (compose::KIND, "instance"),
                    (compose::TEMPLATE, template),
                    (
                        compose::DIRECTORY,
                        directory.to_str().ok_or("invalid template path")?,
                    ),
                    (
                        compose::WORKSPACE,
                        workspace.to_str().ok_or("invalid workspace path")?,
                    ),
                ] {
                    if labels[key].as_str().map(|value| value.replace("$$", "$"))
                        != Some(expected.into())
                    {
                        return Err(format!("unverified instance ownership in {filename}"));
                    }
                }
                if labels[compose::INSTANCE]
                    .as_str()
                    .is_some_and(|instance| instance != name)
                {
                    return Err(format!("conflicting instance name in {filename}"));
                }
            }
            names.insert(name.into());
        }
    }
    Ok(names)
}

fn read_record(path: &Path) -> Result<String, String> {
    if !fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_file()
    {
        return Err(format!(
            "ownership record must be a regular file: {}",
            path.display()
        ));
    }
    read_text(path)
}
