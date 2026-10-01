use std::{fs, path::PathBuf, time::Duration};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::config::{Config, private_file};
use crate::store::environments::validate_instance_name;

pub(super) mod launch;
mod migration;
pub(crate) use migration::prepare;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Journal,
    Startup,
    Ownership,
    Launch,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Journal => "journal",
            Self::Startup => "startup",
            Self::Ownership => "ownership",
            Self::Launch => "launch",
        }
    }
}

fn open(config: &Config) -> Result<Connection, String> {
    let path = config.home.join("settings.sqlite3");
    match private_file(&path, true) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(&path)
                .map_err(|error| error.to_string())?
                .file_type()
                .is_file()
            {
                return Err("Tandem database must be a regular file".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .map_err(|error| error.to_string())?;
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("../../migrations/0005_runtime_records.sql"))
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

pub(super) fn load(config: &Config, name: &str, kind: Kind) -> Result<Option<String>, String> {
    validate_instance_name(name)?;
    open(config)?
        .query_row(
            "SELECT payload FROM runtime_records WHERE namespace = ?1 AND name = ?2 AND kind = ?3",
            params![config.namespace, name.to_ascii_lowercase(), kind.key()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

pub(super) fn list(config: &Config, kind: Kind) -> Result<Vec<(String, String)>, String> {
    let connection = open(config)?;
    let mut statement = connection.prepare(
        "SELECT name, payload FROM runtime_records WHERE namespace = ?1 AND kind = ?2 ORDER BY name"
    ).map_err(|error| error.to_string())?;
    statement
        .query_map(params![config.namespace, kind.key()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn revision(transaction: &Transaction<'_>, config: &Config) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO refresh_revisions(scope, revision) VALUES (?1, 1) ON CONFLICT(scope) DO UPDATE SET revision = revision + 1",
        [format!("instances:{}", config.namespace)],
    )?;
    Ok(())
}

pub(super) fn save(config: &Config, name: &str, kind: Kind, payload: &str) -> Result<(), String> {
    save_many(config, name, &[(kind, payload)])
}

pub(super) fn update_startup(
    config: &Config,
    name: &str,
    operation_id: &str,
    payload: &str,
) -> Result<(), String> {
    validate_instance_name(name)?;
    let mut connection = open(config)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let current: Option<String> = transaction.query_row(
        "SELECT payload FROM runtime_records WHERE namespace = ?1 AND name = ?2 AND kind = 'startup'",
        params![config.namespace,name.to_ascii_lowercase()], |row|row.get(0)
    ).optional().map_err(|error|error.to_string())?;
    let current: serde_json::Value =
        serde_json::from_str(&current.ok_or("startup request missing")?)
            .map_err(|error| error.to_string())?;
    if current["operation"]["id"].as_str() != Some(operation_id) {
        return Err("startup request belongs to another operation".into());
    }
    transaction.execute(
        "UPDATE runtime_records SET payload = ?3 WHERE namespace = ?1 AND name = ?2 AND kind = 'startup'",
        params![config.namespace,name.to_ascii_lowercase(),payload]
    ).map_err(|error|error.to_string())?;
    revision(&transaction, config).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn save_many(
    config: &Config,
    name: &str,
    records: &[(Kind, &str)],
) -> Result<(), String> {
    validate_instance_name(name)?;
    let mut connection = open(config)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    for (kind, payload) in records {
        transaction.execute(
            "INSERT INTO runtime_records(namespace, name, kind, payload) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(namespace, name, kind) DO UPDATE SET payload = excluded.payload",
            params![config.namespace, name.to_ascii_lowercase(), kind.key(), payload],
        ).map_err(|error| error.to_string())?;
    }
    revision(&transaction, config).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn remove(config: &Config, name: &str, kinds: &[Kind]) -> Result<(), String> {
    validate_instance_name(name)?;
    let mut connection = open(config)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    for kind in kinds {
        transaction
            .execute(
                "DELETE FROM runtime_records WHERE namespace = ?1 AND name = ?2 AND kind = ?3",
                params![config.namespace, name.to_ascii_lowercase(), kind.key()],
            )
            .map_err(|error| error.to_string())?;
    }
    revision(&transaction, config).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn directory(config: &Config) -> Result<PathBuf, String> {
    let runtime = config.home.join("runtime");
    private_directory(&runtime)?;
    let path = runtime.join(&config.namespace);
    private_directory(&path)?;
    Ok(path)
}

fn private_directory(path: &std::path::Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    if !fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_dir()
        || fs::canonicalize(path).map_err(|error| error.to_string())? != path
    {
        return Err("runtime directory must be a real directory within Tandem home".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn require_ready(config: &Config) -> Result<(), String> {
    let imported = open(config)?
        .query_row(
            "SELECT 1 FROM runtime_imports WHERE namespace = ?1",
            [&config.namespace],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    imported.ok_or("runtime storage has not been prepared by the parent process".into())
}

#[cfg(test)]
#[path = "tests/runtime_db.rs"]
mod tests;
