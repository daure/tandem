use super::*;
use crate::{
    environments::{compose, events::EventStore, journal, ownership, rules::names, templates},
    store::environments::{Instance, Template},
};
use serde_json::json;
use std::path::Path;

fn fixture() -> (tempfile::TempDir, Config, Template, Instance) {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "storage-test".into(), 9876).unwrap();
    let template = templates::create(&config, "website").unwrap();
    let instance = Instance {
        name: "Review".into(),
        template: "website".into(),
        template_directory: template.directory.clone(),
        workspace: config.workspaces.join("Review").display().to_string(),
        project: config.project("Review"),
        ..Default::default()
    };
    (home, config, template, instance)
}

fn source(config: &Config, template: &Template, name: &str) -> String {
    let mut model = json!({"name":config.project(name), "services":{"web":{"image":"nginx"}}});
    compose::decorate(config, template, name, "", &mut model).unwrap();
    serde_json::to_string(&model).unwrap().replace('$', "$$")
}

fn legacy(config: &Config, template: &Template, instance: &Instance) -> Vec<PathBuf> {
    let root = directory(config).unwrap();
    let journal = root.join("review.json");
    fs::write(&journal, json!({"expected":instance}).to_string()).unwrap();
    let owner = PathBuf::from(&template.directory)
        .join(format!(".tandem-{}-Review.owner.json", config.namespace));
    fs::write(&owner, json!({"namespace":config.namespace,"template":"website","directory":template.directory,"instance":"Review"}).to_string()).unwrap();
    let compose = PathBuf::from(&template.directory)
        .join(format!(".tandem-{}-Review.compose.json", config.namespace));
    fs::write(&compose, source(config, template, &instance.name)).unwrap();
    vec![journal, owner, compose]
}

#[test]
fn instance_records_are_namespace_scoped_and_publish_atomic_revisions() {
    let (_home, config, _template, _instance) = fixture();
    save(&config, "Review", Kind::Journal, "{}").unwrap();
    assert_eq!(
        load(&config, "review", Kind::Journal).unwrap().as_deref(),
        Some("{}")
    );
    let mut other = config.clone();
    other.namespace = "other".into();
    assert!(load(&other, "Review", Kind::Journal).unwrap().is_none());
    let connection = open(&config).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_revision BEFORE UPDATE ON refresh_revisions BEGIN SELECT RAISE(ABORT, 'blocked revision'); END;").unwrap();
    assert!(save(&config, "Review", Kind::Journal, "{\"changed\":true}").is_err());
    assert_eq!(
        load(&config, "Review", Kind::Journal).unwrap().as_deref(),
        Some("{}")
    );
    connection
        .execute_batch("DROP TRIGGER reject_revision;")
        .unwrap();
    save_many(
        &config,
        "Review",
        &[(Kind::Ownership, "{}"), (Kind::Launch, "{}")],
    )
    .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON runtime_records WHEN OLD.kind = 'journal' BEGIN SELECT RAISE(ABORT, 'blocked cleanup'); END;").unwrap();
    assert!(journal::forget(&config, "Review").is_err());
    for kind in [Kind::Journal, Kind::Ownership, Kind::Launch] {
        assert_eq!(
            load(&config, "Review", kind).unwrap().as_deref(),
            Some("{}")
        );
    }
    connection
        .execute_batch("DROP TRIGGER reject_cleanup;")
        .unwrap();
    journal::forget(&config, "Review").unwrap();
    for kind in [Kind::Journal, Kind::Ownership, Kind::Launch] {
        assert!(load(&config, "Review", kind).unwrap().is_none());
    }
}

#[test]
fn journal_ownership_failures_identify_recorded_and_expected_fields() {
    let (_home, config, _template, instance) = fixture();
    save(
        &config,
        &instance.name,
        Kind::Journal,
        &json!({"expected":instance}).to_string(),
    )
    .unwrap();
    assert_eq!(
        journal::recorded(&config, &instance.name).unwrap(),
        Some(instance.clone())
    );
    for field in [
        "name",
        "project",
        "workspace",
        "template_directory",
        "services",
        "name_and_project",
    ] {
        let mut invalid = instance.clone();
        let details = match field {
            "name" => {
                invalid.name = "Other".into();
                "name: recorded=\"Other\", expected=\"Review\"".into()
            }
            "project" => {
                invalid.project = "foreign-review".into();
                format!(
                    "project: recorded=\"foreign-review\", expected={:?}",
                    instance.project
                )
            }
            "workspace" => {
                invalid.workspace = "/foreign/workspaces/Review".into();
                format!(
                    "workspace: recorded=\"/foreign/workspaces/Review\", expected={:?}",
                    config.workspaces.join(&instance.name)
                )
            }
            "template_directory" => {
                invalid.template_directory = "/foreign/templates/website".into();
                format!(
                    "template_directory: recorded=\"/foreign/templates/website\", expected={:?}",
                    config.templates.join(&instance.template)
                )
            }
            "services" => {
                invalid.workspace_only = true;
                invalid.services.push(Default::default());
                "services: recorded count=1, expected count=0 for workspace_only=true".into()
            }
            "name_and_project" => {
                invalid.name = "Other".into();
                invalid.project = "foreign-review".into();
                format!(
                    "name: recorded=\"Other\", expected=\"Review\"; project: recorded=\"foreign-review\", expected={:?}",
                    instance.project
                )
            }
            _ => unreachable!(),
        };
        let payload = json!({"expected":invalid}).to_string();
        save(&config, &instance.name, Kind::Journal, &payload).unwrap();
        assert_eq!(
            journal::recorded(&config, &instance.name).unwrap_err(),
            format!("instance record ownership mismatch for Review: {details}")
        );
        assert_eq!(
            load(&config, &instance.name, Kind::Journal).unwrap(),
            Some(payload)
        );
    }
}

#[test]
fn runtime_writes_commit_while_an_observer_retains_a_read_snapshot() {
    let (_home, config, _template, _instance) = fixture();
    prepare(&config).unwrap();
    save(&config, "Review", Kind::Journal, "original").unwrap();
    let mut reader = open(&config).unwrap();
    let snapshot = reader.transaction().unwrap();
    let original: String = snapshot
        .query_row(
            "SELECT payload FROM runtime_records WHERE kind = 'journal'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original, "original");
    let (sent, received) = std::sync::mpsc::channel();
    let writer_config = config.clone();
    let writer = std::thread::spawn(move || {
        sent.send(save(&writer_config, "Review", Kind::Journal, "updated"))
            .unwrap();
    });
    let committed = received.recv_timeout(Duration::from_secs(1));
    snapshot.rollback().unwrap();
    writer.join().unwrap();
    committed
        .expect("runtime write waited for the observer snapshot")
        .unwrap();
    assert_eq!(
        load(&config, "Review", Kind::Journal).unwrap().as_deref(),
        Some("updated")
    );
}

#[test]
fn startup_updates_require_the_current_operation_and_preserve_deleted_records() {
    let (_home, config, _template, _instance) = fixture();
    let first = json!({"operation":{"id":"1-1"}}).to_string();
    let second = json!({"operation":{"id":"1-2"}}).to_string();
    save(&config, "Review", Kind::Startup, &second).unwrap();
    assert!(
        update_startup(&config, "Review", "1-1", &first)
            .unwrap_err()
            .contains("another operation")
    );
    assert_eq!(
        load(&config, "Review", Kind::Startup).unwrap(),
        Some(second.clone())
    );
    update_startup(&config, "Review", "1-2", &second).unwrap();
    remove(&config, "Review", &[Kind::Startup]).unwrap();
    assert!(
        update_startup(&config, "Review", "1-2", &second)
            .unwrap_err()
            .contains("missing")
    );
    assert!(load(&config, "Review", Kind::Startup).unwrap().is_none());
}

#[test]
fn instance_launch_storage_keeps_recipe_directories_clean_and_repairs_materialization() {
    let (_home, config, template, instance) = fixture();
    let snapshot = launch::Snapshot::new(
        &config,
        &template,
        &instance.name,
        source(&config, &template, &instance.name),
    );
    journal::record_launch(&config, &snapshot, Vec::new(), "description").unwrap();
    ownership::record(
        &config,
        &template.name,
        Path::new(&template.directory),
        &instance.name,
    )
    .unwrap();
    let path = launch::materialize(&config, &instance.name)
        .unwrap()
        .unwrap();
    assert!(path.starts_with(config.home.join("runtime")));
    assert_eq!(fs::read_to_string(&path).unwrap(), snapshot.source);
    fs::write(&path, "{}").unwrap();
    launch::materialize(&config, &instance.name).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), snapshot.source);
    assert_eq!(
        fs::read_dir(&template.directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        ["tandem.json"]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    fs::remove_dir_all(&template.directory).unwrap();
    ownership::verify(&config, &instance).unwrap();
    assert_eq!(
        journal::recorded(&config, &instance.name)
            .unwrap()
            .unwrap()
            .description,
        "description"
    );
    launch::remove(&config, &instance.name).unwrap();
    journal::forget(&config, &instance.name).unwrap();
    assert!(
        load(&config, &instance.name, Kind::Launch)
            .unwrap()
            .is_none()
    );
}

#[test]
fn migration_imports_validated_records_once_with_private_backups() {
    let (_home, config, template, instance) = fixture();
    let paths = legacy(&config, &template, &instance);
    save(
        &config,
        &instance.name,
        Kind::Journal,
        &fs::read_to_string(&paths[0]).unwrap(),
    )
    .unwrap();
    prepare(&config).unwrap();
    let reservation: (String, Option<i64>) = open(&config)
        .unwrap()
        .query_row(
            "SELECT namespace, acceptance_id FROM rule_instance_names WHERE name_key = 'review'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(reservation, (config.namespace.clone(), None));
    assert_eq!(
        journal::recorded(&config, "Review")
            .unwrap()
            .unwrap()
            .workspace,
        instance.workspace
    );
    ownership::verify(&config, &instance).unwrap();
    assert!(
        launch::load(&config, "Review")
            .unwrap()
            .unwrap()
            .manifest
            .is_none()
    );
    assert!(paths.iter().all(|path| !path.exists()));
    let backups = fs::read_dir(directory(&config).unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_dir(&backups[0]).unwrap().count(), 3);
    remove(
        &config,
        "Review",
        &[Kind::Journal, Kind::Ownership, Kind::Launch],
    )
    .unwrap();
    fs::write(&paths[0], json!({"expected":instance}).to_string()).unwrap();
    prepare(&config).unwrap();
    assert!(journal::recorded(&config, "Review").unwrap().is_none());
}

#[test]
fn late_namespace_import_names_survive_purge_and_preserve_existing_attribution() {
    for prior_reservation in [false, true] {
        let (_home, config, template, mut instance) = fixture();
        prepare(&config).unwrap();
        EventStore::open(&config).unwrap();
        let connection = open(&config).unwrap();
        let marker: i64 = connection
            .query_row(
                "SELECT count(*) FROM rule_instance_name_backfill",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, 1);
        if prior_reservation {
            connection
                .execute(
                    "INSERT INTO rule_instance_names VALUES ('review', ?1, 77)",
                    [&config.namespace],
                )
                .unwrap();
        }
        let mut other = config.clone();
        other.namespace = "late-import".into();
        instance.project = other.project(&instance.name);
        fs::create_dir(&instance.workspace).unwrap();
        let originals = legacy(&other, &template, &instance);
        prepare(&other).unwrap();
        require_ready(&other).unwrap();
        assert!(originals.iter().all(|path| !path.exists()));
        let reservation: (String, Option<i64>) = connection
            .query_row(
                "SELECT namespace, acceptance_id FROM rule_instance_names WHERE name_key = 'review'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let expected = if prior_reservation {
            (config.namespace.clone(), Some(77))
        } else {
            (other.namespace.clone(), None)
        };
        assert_eq!(reservation, expected);
        journal::forget(&other, &instance.name).unwrap();
        fs::remove_dir(&instance.workspace).unwrap();
        for kind in [Kind::Journal, Kind::Startup, Kind::Ownership, Kind::Launch] {
            assert!(load(&other, &instance.name, kind).unwrap().is_none());
        }
        assert_eq!(
            names::find_available_name(&connection, &config, "review").unwrap(),
            Some("review-2".into())
        );
        drop(connection);
        prepare(&other).unwrap();
        let reopened = open(&config).unwrap();
        assert_eq!(
            names::find_available_name(&reopened, &config, "review").unwrap(),
            Some("review-2".into())
        );
    }
}

#[test]
fn late_import_reservations_commit_atomically_with_records_and_namespace_marker() {
    let (_home, config, template, mut instance) = fixture();
    prepare(&config).unwrap();
    EventStore::open(&config).unwrap();
    let mut other = config.clone();
    other.namespace = "late-import".into();
    instance.project = other.project(&instance.name);
    let originals = legacy(&other, &template, &instance);
    let connection = open(&config).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_import_name BEFORE INSERT ON rule_instance_names
         WHEN NEW.namespace = 'late-import'
         BEGIN SELECT RAISE(ABORT, 'blocked imported name'); END;",
        )
        .unwrap();
    assert!(
        prepare(&other)
            .unwrap_err()
            .contains("blocked imported name")
    );
    assert!(require_ready(&other).is_err());
    assert!(originals.iter().all(|path| path.exists()));
    for table in [
        "runtime_records",
        "runtime_imports",
        "runtime_import_files",
        "rule_instance_names",
    ] {
        let count: i64 = connection
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE namespace = ?1"),
                [&other.namespace],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    connection
        .execute_batch("DROP TRIGGER reject_import_name;")
        .unwrap();
    prepare(&other).unwrap();
    require_ready(&other).unwrap();
    assert!(originals.iter().all(|path| !path.exists()));
    let records: i64 = connection
        .query_row(
            "SELECT count(*) FROM runtime_records WHERE namespace = ?1 AND name = 'review'",
            [&other.namespace],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(records, 3);
    let names: i64 = connection
        .query_row(
            "SELECT count(*) FROM rule_instance_names WHERE namespace = ?1 AND name_key = 'review'",
            [&other.namespace],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(names, 1);
}

#[test]
fn migration_rejects_foreign_workspaces_owner_only_receipts_and_busy_workers() {
    let (_home, config, template, mut instance) = fixture();
    let paths = legacy(&config, &template, &instance);
    let lock = crate::environments::gateway::lock(&config, "instance-Review").unwrap();
    assert!(prepare(&config).unwrap_err().contains("close old Tandem"));
    assert!(load(&config, "Review", Kind::Journal).unwrap().is_none());
    drop(lock);
    instance.workspace = "/foreign/workspaces/Review".into();
    fs::write(&paths[0], json!({"expected":instance}).to_string()).unwrap();
    assert!(prepare(&config).unwrap_err().contains("ownership mismatch"));
    fs::remove_file(&paths[0]).unwrap();
    fs::remove_file(&paths[2]).unwrap();
    assert!(
        prepare(&config)
            .unwrap_err()
            .contains("lacks local workspace evidence")
    );
    assert!(paths[1].exists());
}

#[test]
fn migration_conflicts_roll_back_and_retry_after_resolution() {
    let (_home, config, template, instance) = fixture();
    let paths = legacy(&config, &template, &instance);
    save(&config, "Review", Kind::Journal, "{}").unwrap();
    assert!(prepare(&config).unwrap_err().contains("conflicting SQLite"));
    assert!(require_ready(&config).is_err());
    assert!(paths.iter().all(|path| path.exists()));
    assert!(load(&config, "Review", Kind::Ownership).unwrap().is_none());
    remove(&config, "Review", &[Kind::Journal]).unwrap();
    prepare(&config).unwrap();
    require_ready(&config).unwrap();
}

#[cfg(unix)]
#[test]
fn migration_preserves_read_only_recipes_and_retries_imported_file_cleanup() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let (_home, config, template, instance) = fixture();
    let paths = legacy(&config, &template, &instance);
    fs::set_permissions(&template.directory, fs::Permissions::from_mode(0o555)).unwrap();
    let result = prepare(&config);
    assert!(result.is_ok(), "{result:?}");
    ownership::verify(&config, &instance).unwrap();
    assert!(paths[1].exists());
    assert!(paths[2].exists());
    fs::set_permissions(&template.directory, fs::Permissions::from_mode(0o755)).unwrap();
    prepare(&config).unwrap();
    assert!(paths.iter().all(|path| !path.exists()));
    assert_eq!(fs::read_dir(&template.directory).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn launch_materialization_rejects_symlink_destinations_and_database_errors_propagate() {
    let (_home, config, template, instance) = fixture();
    let snapshot = launch::Snapshot::new(
        &config,
        &template,
        &instance.name,
        source(&config, &template, &instance.name),
    );
    save(
        &config,
        &instance.name,
        Kind::Launch,
        &snapshot.encode(&config).unwrap(),
    )
    .unwrap();
    let path = launch::path(&config, &instance.name).unwrap();
    let external = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(external.path(), &path).unwrap();
    assert!(
        launch::materialize(&config, &instance.name)
            .unwrap_err()
            .contains("regular file")
    );
    assert_eq!(fs::metadata(external.path()).unwrap().len(), 0);
    fs::remove_file(config.home.join("settings.sqlite3")).unwrap();
    std::os::unix::fs::symlink(external.path(), config.home.join("settings.sqlite3")).unwrap();
    assert!(
        load(&config, "Review", Kind::Journal)
            .unwrap_err()
            .contains("regular file")
    );
}
