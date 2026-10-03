use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use rusqlite::{Transaction, params};

use super::{Definition, Error, decode, encode, rules};
use crate::environments::config::{Config, read_text};

fn storage(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

pub(in crate::environments) fn lock(config: &Config) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(config.home.join("locks/rule-catalog"))
        .map_err(storage)?;
    file.lock().map_err(storage)?;
    Ok(file)
}

fn directory(path: &Path) -> Result<(), Error> {
    fs::create_dir_all(path).map_err(storage)?;
    if !fs::symlink_metadata(path).map_err(storage)?.is_dir()
        || fs::canonicalize(path).map_err(storage)? != path
    {
        return Err(storage(format!(
            "{} must be a real rule catalog directory",
            path.display()
        )));
    }
    Ok(())
}

fn root(config: &Config) -> Result<PathBuf, Error> {
    let templates = config.home.join("templates");
    directory(&templates)?;
    let root = templates.join("rules");
    directory(&root)?;
    Ok(root)
}

fn recipe(definition: &Definition) -> Definition {
    let mut definition = definition.clone();
    definition.enabled = false;
    definition
}

fn read(path: &Path) -> Result<Definition, Error> {
    if !fs::symlink_metadata(path)
        .map_err(storage)?
        .file_type()
        .is_file()
    {
        return Err(storage(format!(
            "{} must be a regular rule file",
            path.display()
        )));
    }
    let text = read_text(path).map_err(storage)?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| Error::Invalid(format!("{}: {error}", path.display())))?;
    if value.get("enabled").is_some() {
        return Err(Error::Invalid(format!(
            "{}: activation is local; omit enabled from rule.json",
            path.display()
        )));
    }
    let definition: Definition = serde_json::from_value(value)
        .map_err(|error| Error::Invalid(format!("{}: {error}", path.display())))?;
    rules::validate(&definition)
        .map_err(|error| Error::Invalid(format!("{}: {error}", path.display())))?;
    Ok(definition)
}

fn list(root: &Path) -> Result<BTreeMap<String, Definition>, Error> {
    let mut definitions = BTreeMap::new();
    for entry in fs::read_dir(root).map_err(storage)? {
        let entry = entry.map_err(storage)?;
        if entry.file_type().map_err(storage)?.is_file() {
            continue;
        }
        let path = entry.path().join("rule.json");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(storage(error)),
            Ok(_) => {}
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::Invalid("rule directory names must be UTF-8".into()))?;
        crate::store::environments::validate_name(&name).map_err(Error::Invalid)?;
        directory(&entry.path())?;
        let definition = read(&path)?;
        if definition.name != name {
            return Err(Error::Invalid(format!(
                "{}: rule name must match its directory",
                path.display()
            )));
        }
        definitions.insert(name, definition);
        if definitions.len() > 100 {
            return Err(Error::Invalid("at most 100 rules per catalog".into()));
        }
    }
    Ok(definitions)
}

fn write(root: &Path, definition: &Definition) -> Result<(), Error> {
    let directory_path = root.join(&definition.name);
    directory(&directory_path)?;
    let path = directory_path.join("rule.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(storage(format!(
                "{} must be a regular rule file",
                path.display()
            )));
        }
        Ok(_) if read(&path)? == recipe(definition) => return Ok(()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(storage(error)),
        _ => {}
    }
    let mut value = serde_json::to_value(definition).map_err(storage)?;
    value
        .as_object_mut()
        .ok_or_else(|| storage("rule must be an object"))?
        .remove("enabled");
    let text = serde_json::to_string_pretty(&value).map_err(storage)? + "\n";
    if text.len() > 262_144 {
        return Err(Error::Invalid("rule.json exceeds 256 KiB".into()));
    }
    let mut file = tempfile::NamedTempFile::new_in(&directory_path).map_err(storage)?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.as_file().sync_all())
        .map_err(storage)?;
    file.persist(&path).map_err(storage)?;
    File::open(directory_path)
        .and_then(|file| file.sync_all())
        .map_err(storage)?;
    Ok(())
}

// Callers hold the shared catalog lock before opening their SQLite write transaction.
pub(in crate::environments) fn synchronize(
    config: &Config,
    transaction: &Transaction<'_>,
) -> Result<(), Error> {
    let root = root(config)?;
    let definitions = list(&root)?;
    let mut statement = transaction.prepare(
        "SELECT name, definition, revision, catalog_present FROM event_rules WHERE namespace = ?1",
    )?;
    let previous = statement
        .query_map([&config.namespace], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, bool>(3)?,
                ),
            ))
        })?
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    for (name, definition) in &definitions {
        let revision = match previous.get(name) {
            Some((text, revision, present)) => {
                if *present && recipe(&decode(text)?) == *definition {
                    continue;
                }
                revision + 1
            }
            None => 1,
        };
        transaction.execute(
            "INSERT INTO event_rules(namespace, name, revision, definition, zellij_session, enabled, catalog_present) VALUES (?1, ?2, ?3, ?4, '', 0, 1)
             ON CONFLICT(namespace, name) DO UPDATE SET revision = excluded.revision, definition = excluded.definition, zellij_session = '', enabled = 0, catalog_present = 1",
            params![config.namespace, name, revision, encode(definition)?],
        )?;
    }
    for (name, (text, revision, present)) in previous {
        if present && !definitions.contains_key(&name) {
            transaction.execute(
                "UPDATE event_rules SET catalog_present = 0, enabled = 0, zellij_session = '', revision = ?3, definition = ?4 WHERE namespace = ?1 AND name = ?2",
                params![config.namespace, name, revision + 1, encode(&recipe(&decode(&text)?))?],
            )?;
        }
    }
    Ok(())
}

pub(super) fn save(config: &Config, definition: &Definition) -> Result<(), Error> {
    write(&root(config)?, definition)
}

#[cfg(test)]
#[path = "../tests/rule_catalog.rs"]
mod tests;
