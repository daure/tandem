use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn reused_connections_read_external_commits_and_refresh_cached_queries_after_schema_changes() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("settings.sqlite3");
    let pool = Arc::new(Pool::new(path.clone()));
    let initialized = AtomicUsize::new(0);
    let initialize = |connection: &Connection| {
        initialized.fetch_add(1, Ordering::Relaxed);
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
    };
    {
        let connection = pool.get(initialize).unwrap();
        connection
            .execute_batch("CREATE TABLE samples(value INTEGER); INSERT INTO samples VALUES (1);")
            .unwrap();
        let value: i64 = connection
            .prepare_cached("SELECT sum(value) FROM samples")
            .unwrap()
            .query_row([], |row| row.get(0))
            .unwrap();
        assert_eq!(value, 1);
    }
    let external = Connection::open(path).unwrap();
    external
        .execute_batch(
            "ALTER TABLE samples ADD COLUMN label TEXT; INSERT INTO samples(value) VALUES (2);",
        )
        .unwrap();
    let connection = Arc::clone(&pool).get(initialize).unwrap();
    let value: i64 = connection
        .prepare_cached("SELECT sum(value) FROM samples")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert_eq!(value, 3);
    assert_eq!(initialized.load(Ordering::Relaxed), 1);
    assert_eq!(
        connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5_000
    );
}

#[test]
fn readers_keep_consistent_snapshots_without_blocking_independent_writers() {
    let home = tempfile::tempdir().unwrap();
    let pool = Arc::new(Pool::new(home.path().join("settings.sqlite3")));
    let initialize =
        |connection: &Connection| connection.execute_batch("PRAGMA journal_mode = WAL;");
    let mut reader = pool.get(initialize).unwrap();
    reader
        .execute_batch("CREATE TABLE samples(value INTEGER); INSERT INTO samples VALUES (1);")
        .unwrap();
    let transaction = reader.transaction().unwrap();
    let read = |connection: &Connection| {
        connection
            .prepare_cached("SELECT sum(value) FROM samples")
            .unwrap()
            .query_row([], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    assert_eq!(read(&transaction), 1);
    let writer = pool.get(initialize).unwrap();
    writer
        .execute("INSERT INTO samples VALUES (2)", [])
        .unwrap();
    assert_eq!(read(&transaction), 1);
    transaction.commit().unwrap();
    assert_eq!(read(&reader), 3);
}

#[test]
fn abandoned_transactions_roll_back_and_release_their_locks() {
    let home = tempfile::tempdir().unwrap();
    let pool = Arc::new(Pool::new(home.path().join("settings.sqlite3")));
    {
        let connection = pool.get(|_| Ok(())).unwrap();
        connection.execute_batch("CREATE TABLE samples(value INTEGER); BEGIN IMMEDIATE; INSERT INTO samples VALUES (1);").unwrap();
    }
    let connection = pool.get(|_| Ok(())).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM samples", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    connection
        .execute("INSERT INTO samples VALUES (2)", [])
        .unwrap();
}

#[test]
fn replacing_a_database_discards_idle_connections_to_the_previous_file() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("settings.sqlite3");
    let pool = Arc::new(Pool::new(path.clone()));
    {
        let connection = pool.get(|_| Ok(())).unwrap();
        connection
            .execute_batch("CREATE TABLE samples(value INTEGER); INSERT INTO samples VALUES (1);")
            .unwrap();
        connection
            .prepare_cached("SELECT value FROM samples")
            .unwrap();
    }
    let replacement = home.path().join("replacement.sqlite3");
    {
        let connection = Connection::open(&replacement).unwrap();
        connection
            .execute_batch("CREATE TABLE samples(value INTEGER); INSERT INTO samples VALUES (2);")
            .unwrap();
    }
    fs::rename(replacement, path).unwrap();
    let connection = pool.get(|_| Ok(())).unwrap();
    assert_eq!(
        connection
            .prepare_cached("SELECT value FROM samples")
            .unwrap()
            .query_row([], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn concurrent_checkouts_remain_independent_and_idle_connections_are_bounded() {
    let home = tempfile::tempdir().unwrap();
    let pool = Arc::new(Pool::new(home.path().join("settings.sqlite3")));
    let leases = (0..8)
        .map(|_| pool.get(|_| Ok(())).unwrap())
        .collect::<Vec<_>>();
    drop(leases);
    assert_eq!(pool.idle.lock().unwrap().len(), IDLE_LIMIT);
}

#[test]
fn changing_database_permissions_reinitializes_the_connection() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("settings.sqlite3");
    let pool = Arc::new(Pool::new(path.clone()));
    let initialized = AtomicUsize::new(0);
    let initialize = |_: &Connection| {
        initialized.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    drop(pool.get(initialize).unwrap());
    let permissions = fs::metadata(&path).unwrap().permissions().mode();
    fs::set_permissions(&path, fs::Permissions::from_mode(permissions ^ 0o040)).unwrap();
    drop(pool.get(initialize).unwrap());
    assert_eq!(initialized.load(Ordering::Relaxed), 2);
}
