use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::{Config, Kind, launch};
use crate::{
    environments::{
        config::{private_file, read_text},
        gateway, journal, ownership, startup,
    },
    store::environments::{Instance, validate_instance_name, validate_name},
};

struct Import {
    path: PathBuf,
    original: String,
    payload: String,
    name: String,
    kind: Kind,
}

pub(crate) fn prepare(config: &Config) -> Result<(), String> {
    let progress: crate::environments::command::Progress = std::sync::Arc::new(|_| {});
    let _gate = gateway::lock_until(
        config,
        "runtime-migration",
        std::time::Instant::now() + std::time::Duration::from_secs(5),
        &progress,
    )?;
    let mut connection = super::open(config)?;
    if connection
        .query_row(
            "SELECT 1 FROM runtime_imports WHERE namespace = ?1",
            [&config.namespace],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        cleanup_imported(config, &connection)?;
        return Ok(());
    }

    let mut imports = Vec::new();
    let mut local = BTreeMap::<String, Instance>::new();
    let mut locks = BTreeSet::new();
    let directory = super::directory(config)?;
    for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let (key, kind) = if let Some(name) = filename.strip_suffix(".startup.json") {
            (name, Kind::Startup)
        } else if let Some(name) = filename.strip_suffix(".json") {
            (name, Kind::Journal)
        } else {
            continue;
        };
        validate_instance_name(key)?;
        let original = legacy_text(&path)?;
        let name = match kind {
            Kind::Startup => {
                let record = startup::decode(key, &original)?;
                locks.insert(format!("startup-{}", record.operation.id));
                record.operation.name
            }
            Kind::Journal => {
                let expected = journal::validate_legacy(config, key, &original)?;
                if let Some(instance) = expected {
                    let name = instance.name.clone();
                    local.insert(key.into(), instance);
                    name
                } else {
                    let value: Value =
                        serde_json::from_str(&original).map_err(|error| error.to_string())?;
                    value["activity"]["name"].as_str().unwrap_or(key).to_owned()
                }
            }
            _ => unreachable!(),
        };
        locks.insert(format!("instance-{name}"));
        imports.push(Import {
            path,
            payload: original.clone(),
            original,
            name,
            kind,
        });
    }

    let prefix = format!(".tandem-{}-", config.namespace);
    let mut owners = Vec::new();
    let mut launches = BTreeMap::new();
    for entry in fs::read_dir(&config.templates).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let template = entry.file_name().to_string_lossy().into_owned();
        if validate_name(&template).is_err() {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            continue;
        }
        let directory = entry.path();
        if fs::canonicalize(&directory).map_err(|error| error.to_string())? != directory {
            return Err("legacy template directory must be a real directory".into());
        }
        for file in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let path = file.map_err(|error| error.to_string())?.path();
            let Some(suffix) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix(&prefix))
            else {
                continue;
            };
            let (name, kind) = if let Some(name) = suffix.strip_suffix(".compose.json") {
                (name, Kind::Launch)
            } else if let Some(name) = suffix.strip_suffix(".owner.json") {
                (name, Kind::Ownership)
            } else {
                continue;
            };
            validate_instance_name(name)?;
            let name = name.to_owned();
            let original = legacy_text(&path)?;
            locks.insert(format!("instance-{name}"));
            match kind {
                Kind::Launch => {
                    let snapshot =
                        launch::Snapshot::legacy(config, &template, &name, original.clone())?;
                    if let Some(instance) = local.get(&name.to_ascii_lowercase())
                        && (instance.name != name
                            || instance.template != template
                            || instance.workspace_only)
                    {
                        return Err("legacy launch conflicts with local instance identity".into());
                    }
                    if launches
                        .insert(name.to_ascii_lowercase(), template.clone())
                        .is_some()
                    {
                        return Err("legacy launch names collide".into());
                    }
                    imports.push(Import {
                        path,
                        payload: snapshot.encode(config)?,
                        original,
                        name,
                        kind,
                    });
                }
                Kind::Ownership => owners.push((path, original, name, template.clone())),
                _ => unreachable!(),
            }
        }
    }
    for (path, original, name, template) in owners {
        let corroborated = local
            .get(&name.to_ascii_lowercase())
            .is_some_and(|instance| instance.name == name && instance.template == template)
            || launches.get(&name.to_ascii_lowercase()) == Some(&template);
        let payload = ownership::import_legacy(config, &template, &name, &original, corroborated)?;
        imports.push(Import {
            path,
            original,
            payload,
            name,
            kind: Kind::Ownership,
        });
    }
    let _locks = locks
        .iter()
        .map(|name| gateway::lock(config, name))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!("close old Tandem clients and workers before upgrading: {error}")
        })?;
    for import in &imports {
        if legacy_text(&import.path)? != import.original {
            return Err(
                "legacy state changed during import; close old Tandem clients and retry".into(),
            );
        }
    }
    if !imports.is_empty() {
        backup(&directory, &imports)?;
    }
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    for import in &imports {
        transaction
            .execute(
                "INSERT INTO runtime_import_files(namespace,path,original) VALUES (?1,?2,?3)",
                params![
                    config.namespace,
                    import.path.to_str().ok_or("legacy path must be UTF-8")?,
                    import.original
                ],
            )
            .map_err(|error| error.to_string())?;
        let existing: Option<String> = transaction.query_row(
            "SELECT payload FROM runtime_records WHERE namespace = ?1 AND name = ?2 AND kind = ?3",
            params![config.namespace, import.name.to_ascii_lowercase(), import.kind.key()], |row| row.get(0)
        ).optional().map_err(|error| error.to_string())?;
        if let Some(existing) = existing {
            let existing: Value =
                serde_json::from_str(&existing).map_err(|error| error.to_string())?;
            let imported: Value =
                serde_json::from_str(&import.payload).map_err(|error| error.to_string())?;
            if existing != imported {
                return Err(format!(
                    "conflicting SQLite and legacy {} record for {}",
                    import.kind.key(),
                    import.name
                ));
            }
        } else {
            transaction.execute(
                "INSERT INTO runtime_records(namespace, name, kind, payload) VALUES (?1, ?2, ?3, ?4)",
                params![config.namespace, import.name.to_ascii_lowercase(), import.kind.key(), import.payload]
            ).map_err(|error| error.to_string())?;
        }
    }
    transaction
        .execute(
            "INSERT INTO runtime_imports(namespace, imported_at) VALUES (?1, ?2)",
            params![config.namespace, journal::now()],
        )
        .map_err(|error| error.to_string())?;
    super::revision(&transaction, config).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    // Originals remain available in private backups; a read-only checkout never blocks import.
    cleanup_imported(config, &connection)?;
    Ok(())
}

fn legacy_text(path: &Path) -> Result<String, String> {
    if !fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_file()
    {
        return Err(format!(
            "legacy state must be a regular file: {}",
            path.display()
        ));
    }
    read_text(path)
}

fn backup(root: &Path, imports: &[Import]) -> Result<(), String> {
    let directory = tempfile::Builder::new()
        .prefix("legacy-import-")
        .tempdir_in(root)
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    for (index, import) in imports.iter().enumerate() {
        let path = directory.path().join(format!(
            "{index}-{}-{}.json",
            import.name,
            import.kind.key()
        ));
        let mut file = private_file(&path, true).map_err(|error| error.to_string())?;
        file.write_all(import.original.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| error.to_string())?;
    }
    let _ = directory.keep();
    Ok(())
}

fn cleanup_imported(config: &Config, connection: &rusqlite::Connection) -> Result<(), String> {
    let files = connection
        .prepare("SELECT path, original FROM runtime_import_files WHERE namespace = ?1")
        .map_err(|error| error.to_string())?
        .query_map([&config.namespace], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (path, original) in files {
        let result = (|| {
            let path = Path::new(&path);
            let parent = path.parent().ok_or("invalid legacy state path")?;
            if !path.starts_with(&config.templates)
                && !path.starts_with(config.home.join("runtime"))
            {
                return Err("legacy state path escapes Tandem directories".to_owned());
            }
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
                Ok(_) => {}
            }
            if fs::canonicalize(parent).map_err(|error| error.to_string())? != parent {
                return Err("legacy state parent must be a real directory".into());
            }
            if legacy_text(path)? != original {
                return Err("legacy state changed after import; preserving it".into());
            }
            fs::remove_file(path).map_err(|error| error.to_string())
        })();
        match result {
            Ok(()) => {
                connection
                    .execute(
                        "DELETE FROM runtime_import_files WHERE namespace = ?1 AND path = ?2",
                        params![config.namespace, path],
                    )
                    .map_err(|error| error.to_string())?;
            }
            Err(error) => crate::diagnostics::record_error(
                "cannot remove imported legacy state; SQLite is authoritative",
                &std::io::Error::other(error),
            ),
        }
    }
    Ok(())
}
