use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    config::Config,
    runtime_db::{self, Kind},
};
use crate::store::environments::{Instance, validate_instance_name, validate_name};

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Owner {
    namespace: String,
    template: String,
    directory: PathBuf,
    instance: String,
    #[serde(default)]
    workspace: PathBuf,
}

fn expected(
    config: &Config,
    template: &str,
    directory: &Path,
    name: &str,
) -> Result<Owner, String> {
    validate_name(template)?;
    validate_instance_name(name)?;
    if directory != config.templates.join(template) {
        return Err("template directory does not match instance ownership".into());
    }
    Ok(Owner {
        namespace: config.namespace.clone(),
        template: template.into(),
        directory: directory.into(),
        instance: name.into(),
        workspace: config.workspaces.join(name),
    })
}

pub(super) fn record(
    config: &Config,
    template: &str,
    directory: &Path,
    name: &str,
) -> Result<(), String> {
    let owner = expected(config, template, directory, name)?;
    if let Some(text) = runtime_db::load(config, name, Kind::Ownership)? {
        let existing: Owner = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if existing != owner {
            return Err(format!("conflicting instance ownership: {name}"));
        }
        return Ok(());
    }
    runtime_db::save(
        config,
        name,
        Kind::Ownership,
        &serde_json::to_string(&owner).map_err(|error| error.to_string())?,
    )
}

pub(super) fn verify(config: &Config, instance: &Instance) -> Result<(), String> {
    let expected = expected(
        config,
        &instance.template,
        Path::new(&instance.template_directory),
        &instance.name,
    )?;
    let text = runtime_db::load(config, &instance.name, Kind::Ownership)?.ok_or_else(|| {
        format!(
            "cannot verify cleanup ownership for {}: ownership record missing",
            instance.name
        )
    })?;
    let owner: Owner = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    if owner != expected || Path::new(&instance.workspace) != expected.workspace {
        return Err(format!(
            "conflicting cleanup ownership for {}",
            instance.name
        ));
    }
    Ok(())
}

pub(super) fn instances(
    config: &Config,
    template: &str,
    directory: &Path,
) -> Result<BTreeSet<String>, String> {
    validate_name(template)?;
    let mut names = BTreeSet::new();
    for (key, text) in runtime_db::list(config, Kind::Ownership)? {
        let owner: Owner = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if owner != expected(config, &owner.template, &owner.directory, &owner.instance)?
            || owner.instance.to_ascii_lowercase() != key
        {
            return Err("conflicting instance ownership record".into());
        }
        if owner.template == template && owner.directory == directory {
            names.insert(owner.instance);
        }
    }
    Ok(names)
}

pub(super) fn import_legacy(
    config: &Config,
    template: &str,
    name: &str,
    text: &str,
    corroborated: bool,
) -> Result<String, String> {
    let mut owner: Owner = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if owner.workspace.as_os_str().is_empty() {
        if !corroborated {
            return Err(format!(
                "legacy ownership for {name} lacks local workspace evidence"
            ));
        }
        owner.workspace = config.workspaces.join(name);
    }
    if owner != expected(config, template, &config.templates.join(template), name)? {
        return Err(format!("conflicting legacy ownership for {name}"));
    }
    serde_json::to_string(&owner).map_err(|error| error.to_string())
}
