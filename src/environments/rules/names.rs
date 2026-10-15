use std::{collections::HashSet, fs};

use rusqlite::{Connection, Transaction, params};

use crate::store::{environments::validate_instance_name, events::Error, rules::Acceptance};

use super::{Config, decode};

pub(in crate::environments) fn initialize(transaction: &Transaction<'_>) -> Result<(), Error> {
    let initialized: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM rule_instance_name_backfill WHERE id = 1)",
        [],
        |row| row.get(0),
    )?;
    if initialized {
        return Ok(());
    }
    let mut statement = transaction.prepare(
        "SELECT a.payload, e.namespace FROM rule_acceptances a
         JOIN event_attempts p ON p.id = a.attempt_id
         JOIN events e ON e.sequence = p.event_sequence ORDER BY a.id",
    )?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (payload, namespace) = row?;
        let acceptance: Acceptance = decode(&payload)?;
        backfill_name(
            transaction,
            &acceptance.instance,
            &namespace,
            Some(acceptance.id),
        )?;
    }
    let mut statement = transaction.prepare("SELECT name, namespace FROM runtime_records")?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (name, namespace) = row?;
        backfill_name(transaction, &name, &namespace, None)?;
    }
    transaction.execute("INSERT INTO rule_instance_name_backfill(id) VALUES (1)", [])?;
    Ok(())
}

pub(in crate::environments) fn backfill_name(
    transaction: &Transaction<'_>,
    name: &str,
    namespace: &str,
    id: Option<i64>,
) -> Result<(), Error> {
    validate_instance_name(name).map_err(Error::Invalid)?;
    transaction.execute(
        "INSERT INTO rule_instance_names(name_key, namespace, acceptance_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(name_key) DO NOTHING",
        params![name.to_ascii_lowercase(), namespace, id],
    )?;
    Ok(())
}

fn workspace_paths(
    connection: &Connection,
    config: &Config,
) -> Result<Vec<std::path::PathBuf>, Error> {
    let root = &config.workspaces;
    let metadata = fs::symlink_metadata(root).map_err(storage)?;
    let canonical = fs::canonicalize(root).map_err(storage)?;
    if !metadata.file_type().is_dir() || canonical != *root || !canonical.starts_with(&config.home)
    {
        return Err(Error::Storage(
            "workspaces require a real contained directory".into(),
        ));
    }
    let recorded = crate::environments::runtime_db::workspace_paths_from(connection)
        .map_err(Error::Storage)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(root).map_err(storage)? {
        let entry = entry.map_err(storage)?;
        let path = entry.path();
        let grouped = !recorded.contains(&path)
            && (recorded
                .iter()
                .any(|workspace| workspace.parent() == Some(path.as_path()))
                || config.templates.join(entry.file_name()).is_dir());
        if grouped && entry.file_type().map_err(storage)?.is_dir() {
            for child in fs::read_dir(&path).map_err(storage)? {
                paths.push(child.map_err(storage)?.path());
            }
        } else {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn workspace_entries(connection: &Connection, config: &Config) -> Result<HashSet<String>, Error> {
    Ok(workspace_paths(connection, config)?
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .collect())
}

fn storage(error: std::io::Error) -> Error {
    Error::Storage(error.to_string())
}

fn runtime_occupied(connection: &Connection, key: &str) -> Result<bool, Error> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_records WHERE lower(name) = ?1)",
        [key],
        |row| row.get(0),
    )?)
}

pub(in crate::environments) fn find_available_name(
    connection: &Connection,
    config: &Config,
    base: &str,
) -> Result<Option<String>, Error> {
    validate_instance_name(base).map_err(Error::Invalid)?;
    let entries = workspace_entries(connection, config)?;
    for number in 1..=1024 {
        let candidate = if number == 1 {
            base.to_owned()
        } else {
            let suffix = format!("-{number}");
            let prefix = &base[..base.len().min(40 - suffix.len())];
            format!("{}{suffix}", prefix.trim_end_matches('-'))
        };
        validate_instance_name(&candidate).map_err(Error::Invalid)?;
        let key = candidate.to_ascii_lowercase();
        let reserved: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM rule_instance_names WHERE name_key = ?1)",
            [&key],
            |row| row.get(0),
        )?;
        if !entries.contains(&key) && !reserved && !runtime_occupied(connection, &key)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

pub(super) fn reserve_name(
    transaction: &Transaction<'_>,
    config: &Config,
    base: &str,
    id: i64,
) -> Result<Option<String>, Error> {
    let name = find_available_name(transaction, config, base)?;
    if let Some(name) = &name {
        transaction.execute(
            "INSERT INTO rule_instance_names(name_key, namespace, acceptance_id) VALUES (?1, ?2, ?3)",
            params![name.to_ascii_lowercase(), config.namespace, id],
        )?;
    }
    Ok(name)
}

pub(super) fn ensure_fresh(
    connection: &Connection,
    config: &Config,
    name: &str,
) -> Result<(), Error> {
    validate_instance_name(name).map_err(Error::Invalid)?;
    let key = name.to_ascii_lowercase();
    if workspace_entries(connection, config)?.contains(&key) || runtime_occupied(connection, &key)?
    {
        return Err(Error::Conflict(
            "assigned instance name is already in use; inspect before retrying".into(),
        ));
    }
    Ok(())
}

pub(super) fn ensure_retry(
    connection: &Connection,
    config: &Config,
    name: &str,
    template: &str,
    origin: &str,
) -> Result<(), Error> {
    validate_instance_name(name).map_err(Error::Invalid)?;
    let key = name.to_ascii_lowercase();
    let paths = workspace_paths(connection, config)?;
    let matching: Vec<_> = paths
        .iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|entry| entry.to_string_lossy().eq_ignore_ascii_case(name))
        })
        .collect();
    let workspace = !matching.is_empty();
    if matching
        .iter()
        .any(|path| !crate::environments::workspace::matches(config, template, name, path))
    {
        return Err(Error::Conflict(
            "assigned workspace uses a different case or template; inspect before retrying".into(),
        ));
    }
    let foreign: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_records WHERE lower(name) = ?1 AND namespace != ?2)",
        params![key, config.namespace],
        |row| row.get(0),
    )?;
    if foreign {
        return Err(Error::Conflict(
            "assigned instance belongs to another namespace".into(),
        ));
    }
    let mut statement = connection.prepare(
        "SELECT payload FROM runtime_records WHERE lower(name) = ?1 AND namespace = ?2 AND kind = 'startup'",
    )?;
    let startups = statement
        .query_map(params![key, config.namespace], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    match startups.as_slice() {
        [payload] => {
            let record =
                crate::environments::startup::decode(name, payload).map_err(Error::Storage)?;
            if record.origin_operation_id() != origin
                || record.operation.template.as_deref() != Some(template)
            {
                return Err(Error::Conflict("assigned instance has a different startup operation or template; inspect before retrying".into()));
            }
        }
        [] if workspace || runtime_occupied(connection, &key)? => {
            return Err(Error::Conflict(
                "assigned instance has no verifiable startup lineage; inspect before retrying"
                    .into(),
            ));
        }
        [] => {}
        _ => {
            return Err(Error::Conflict(
                "assigned instance has ambiguous startup lineage".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/names.rs"]
mod tests;
