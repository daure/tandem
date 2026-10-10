use super::*;
use crate::environments::events::EventStore;

fn fixture() -> (tempfile::TempDir, Config, Connection) {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "names".into(), 9876).unwrap();
    let events = EventStore::open(&config).unwrap();
    let connection = events.connection().unwrap();
    (home, config, connection)
}

#[test]
fn searches_all_namespace_records_and_every_case_insensitive_workspace_entry() {
    let (_home, config, connection) = fixture();
    connection
        .execute(
            "INSERT INTO runtime_records VALUES ('foreign', 'Review', 'ownership', '{}')",
            [],
        )
        .unwrap();
    fs::write(config.workspaces.join("REVIEW-2"), "occupied").unwrap();
    std::os::unix::fs::symlink("missing", config.workspaces.join("Review-3")).unwrap();
    fs::create_dir(config.workspaces.join("review-4")).unwrap();
    assert_eq!(
        find_available_name(&connection, &config, "review").unwrap(),
        Some("review-5".into())
    );
    assert!(ensure_fresh(&connection, &config, "review").is_err());
    assert!(
        ensure_retry(&connection, &config, "review", "blank", "1-2-3")
            .unwrap_err()
            .to_string()
            .contains("another namespace")
    );
    assert!(ensure_retry(&connection, &config, "review-2", "blank", "1-2-3").is_err());
    if unsafe { libc::geteuid() } != 0 {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::metadata(&config.workspaces).unwrap().permissions();
        fs::set_permissions(&config.workspaces, fs::Permissions::from_mode(0o000)).unwrap();
        let failed = find_available_name(&connection, &config, "free").is_err();
        fs::set_permissions(&config.workspaces, permissions).unwrap();
        assert!(failed);
    }
    fs::remove_dir_all(&config.workspaces).unwrap();
    std::os::unix::fs::symlink(&config.templates, &config.workspaces).unwrap();
    assert!(find_available_name(&connection, &config, "free").is_err());
}

#[test]
fn reservation_checks_final_truncated_suffix_and_stops_at_1024() {
    let (_home, config, mut connection) = fixture();
    let base = "a".repeat(40);
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(
        reserve_name(&transaction, &config, &base, 1).unwrap(),
        Some(base.clone())
    );
    let second = format!("{}-2", "a".repeat(38));
    fs::write(config.workspaces.join(&second), "occupied").unwrap();
    let third = reserve_name(&transaction, &config, &base, 2)
        .unwrap()
        .unwrap();
    assert_eq!(third, format!("{}-3", "a".repeat(38)));
    transaction.commit().unwrap();
    for number in 1..=1024 {
        let name = if number == 1 {
            "full".into()
        } else {
            format!("full-{number}")
        };
        connection
            .execute(
                "INSERT INTO rule_instance_names VALUES (?1, 'other', NULL)",
                [name],
            )
            .unwrap();
    }
    assert_eq!(
        find_available_name(&connection, &config, "full").unwrap(),
        None
    );
    connection
        .execute(
            "DELETE FROM rule_instance_names WHERE name_key = 'full-1024'",
            [],
        )
        .unwrap();
    assert_eq!(
        find_available_name(&connection, &config, "full").unwrap(),
        Some("full-1024".into())
    );
}

#[test]
fn one_time_backfill_is_atomic_and_keeps_home_wide_runtime_names() {
    let (_home, config, mut connection) = fixture();
    connection
        .execute("DELETE FROM rule_instance_name_backfill", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO runtime_records VALUES ('other', 'Legacy-', 'ownership', '{}')",
            [],
        )
        .unwrap();
    {
        let transaction = connection.transaction().unwrap();
        initialize(&transaction).unwrap();
    }
    let count: i64 = connection
        .query_row("SELECT count(*) FROM rule_instance_names", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    EventStore::open(&config).unwrap();
    connection
        .execute("DELETE FROM runtime_records", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO runtime_records VALUES ('other', 'Later', 'ownership', '{}')",
            [],
        )
        .unwrap();
    EventStore::open(&config).unwrap();
    assert_eq!(
        find_available_name(&connection, &config, "Legacy-").unwrap(),
        Some("Legacy-2".into())
    );
    connection
        .execute("DELETE FROM runtime_records", [])
        .unwrap();
    assert_eq!(
        find_available_name(&connection, &config, "Later").unwrap(),
        Some("Later".into())
    );
}

#[test]
fn concurrent_connections_reserve_distinct_permanent_names() {
    let (_home, config, _connection) = fixture();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (1..=2)
        .map(|id| {
            let config = config.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let events = EventStore::open(&config).unwrap();
                let mut connection = events.connection().unwrap();
                barrier.wait();
                let transaction = connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
                let name = reserve_name(&transaction, &config, "shared", id)
                    .unwrap()
                    .unwrap();
                transaction.commit().unwrap();
                name
            })
        })
        .collect();
    let mut names: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["shared", "shared-2"]);
}

#[test]
fn retries_require_matching_origin_and_template_when_resources_remain() {
    let (_home, config, connection) = fixture();
    let operation = crate::environments::Environments::new(config.clone())
        .begin("create_instance", "retry", Some("blank".into()))
        .unwrap();
    let record = crate::environments::startup::Record {
        operation,
        origin_operation_id: Some("1-2-3".into()),
        description: Some("frozen".into()),
        branch_instances: false,
        start_instance: false,
        preserve_opencode_history: false,
        kind: crate::store::environments::StartupKind::Cold,
        started_at: 1,
        timeout: 60,
        owner_pid: 0,
        workspace_ready: false,
        opencode_requested: false,
        opencode_result: None,
        services: vec![],
    };
    let payload = serde_json::to_string(&record).unwrap();
    connection
        .execute(
            "INSERT INTO runtime_records VALUES (?1, 'retry', 'startup', ?2)",
            params![config.namespace, payload],
        )
        .unwrap();
    ensure_retry(&connection, &config, "retry", "blank", "1-2-3").unwrap();
    assert!(ensure_retry(&connection, &config, "retry", "blank", "4-5-6").is_err());
    assert!(ensure_retry(&connection, &config, "retry", "other", "1-2-3").is_err());
    connection
        .execute(
            "INSERT INTO runtime_records VALUES (?1, 'Retry', 'startup', ?2)",
            params![config.namespace, payload],
        )
        .unwrap();
    assert!(ensure_retry(&connection, &config, "retry", "blank", "1-2-3").is_err());
    fs::create_dir(config.workspaces.join("retry")).unwrap();
    connection
        .execute("DELETE FROM runtime_records", [])
        .unwrap();
    assert!(ensure_retry(&connection, &config, "retry", "blank", "1-2-3").is_err());
    fs::remove_dir(config.workspaces.join("retry")).unwrap();
    ensure_retry(&connection, &config, "retry", "blank", "1-2-3").unwrap();
}

#[test]
fn ledger_rejects_non_ascii_reserved_and_malformed_keys() {
    let (_home, _config, connection) = fixture();
    for name in [
        "",
        "Review",
        "gateway",
        "-bad",
        "bad/name",
        "é",
        "a\0b",
        &"a".repeat(41),
    ] {
        assert!(
            connection
                .execute(
                    "INSERT INTO rule_instance_names VALUES (?1, 'test', NULL)",
                    [name],
                )
                .is_err(),
            "{name:?}"
        );
    }
}
