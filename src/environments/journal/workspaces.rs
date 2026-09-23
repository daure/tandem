use std::path::Path;

use super::{Config, Instance, read, write};
use crate::store::environments::{
    RepositoryCheckout, Template, validate_instance_name, validate_name,
};

pub(in crate::environments) fn recorded(
    config: &Config,
    name: &str,
) -> Result<Option<Instance>, String> {
    let expected = read(config, name)?.expected;
    if let Some(instance) = &expected {
        validate_instance_name(&instance.name)?;
        validate_name(&instance.template)?;
        if instance.name != name
            || instance.project != config.project(name)
            || Path::new(&instance.workspace) != config.workspaces.join(name)
            || Path::new(&instance.template_directory) != config.templates.join(&instance.template)
            || instance.workspace_only && !instance.services.is_empty()
        {
            return Err(format!("instance record ownership mismatch for {name}"));
        }
    }
    Ok(expected)
}

pub(in crate::environments) fn prepare(
    config: &Config,
    template: &Template,
    name: &str,
    description: Option<&str>,
) -> Result<(), String> {
    let existing = recorded(config, name)?;
    if existing.as_ref().is_some_and(|instance| {
        instance.template != template.name || instance.workspace_only != template.workspace_only()
    }) {
        return Err(
            "instance belongs to another template or execution kind; use a new instance name"
                .into(),
        );
    }
    let mut record = read(config, name)?;
    let instance = record.expected.get_or_insert_with(|| Instance {
        name: name.into(),
        description: description.unwrap_or_default().into(),
        template: template.name.clone(),
        template_directory: template.directory.clone(),
        workspace: config.workspaces.join(name).display().to_string(),
        project: config.project(name),
        workspace_only: template.workspace_only(),
        ..Default::default()
    });
    if let Some(description) = description {
        instance.description = description.into();
    }
    instance.runtime.workspace_ready = false;
    record.repositories.clear();
    write(config, name, &record)
}

pub(in crate::environments) fn checkout(
    config: &Config,
    name: &str,
    checkout: RepositoryCheckout,
) -> Result<(), String> {
    let mut record = read(config, name)?;
    record
        .repositories
        .retain(|repository| repository.target != checkout.target);
    record.repositories.push(checkout);
    record.repositories.sort_by(|a, b| a.target.cmp(&b.target));
    write(config, name, &record)
}

pub(in crate::environments) fn workspace_ready(config: &Config, name: &str) -> Result<(), String> {
    let mut record = read(config, name)?;
    record
        .expected
        .as_mut()
        .ok_or("instance record missing")?
        .runtime
        .workspace_ready = true;
    write(config, name, &record)
}

pub(in crate::environments) fn workspace_instance(
    config: &Config,
    name: &str,
) -> Result<Option<Instance>, String> {
    let Some(mut instance) = recorded(config, name)?.filter(|instance| instance.workspace_only)
    else {
        return Ok(None);
    };
    super::enrich(config, std::slice::from_mut(&mut instance))?;
    Ok(Some(instance))
}

pub(in crate::environments) fn workspaces(config: &Config) -> Result<Vec<Instance>, String> {
    let mut instances = Vec::new();
    for entry in std::fs::read_dir(super::directory(config)?).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let key = path
            .file_stem()
            .ok_or("invalid instance record")?
            .to_string_lossy();
        let record = read(config, &key)?;
        if let Some(expected) = record.expected.filter(|instance| instance.workspace_only) {
            if expected.name.to_ascii_lowercase() != key {
                return Err("instance record name mismatch".into());
            }
            if let Some(instance) = recorded(config, &expected.name)? {
                instances.push(instance);
            }
        }
    }
    super::enrich(config, &mut instances)?;
    Ok(instances)
}

pub(in crate::environments) fn forget(config: &Config, name: &str) -> Result<(), String> {
    match std::fs::remove_file(super::path(config, name)?) {
        Ok(()) => super::publish(config),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}
