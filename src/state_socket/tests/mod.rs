use std::{fs, os::unix::fs::PermissionsExt};

use serde_json::{Value, json};

use super::*;

mod bursts;

fn fixture() -> (tempfile::TempDir, Config, Arc<Store>) {
    let home = tempfile::Builder::new()
        .prefix("tandem-socket-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let config = Config::at(home.path().to_owned(), "socket-test".into(), 9876).unwrap();
    crate::environments::runtime_db::prepare(&config).unwrap();
    let store = Arc::new(Store::new(&config).unwrap());
    (home, config, store)
}

async fn next(reader: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    let count = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert!(count > 0, "state connection closed");
    serde_json::from_str(&line).unwrap()
}

#[test]
fn connecting_receives_a_snapshot_then_full_change_events() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (_home, config, store) = fixture();
        let socket = Listener::bind(&config).unwrap();
        let path = socket_path(&config).unwrap();
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
        let server = tokio::spawn(serve(store.clone(), socket));
        let mut reader = BufReader::new(UnixStream::connect(&path).await.unwrap());
        let snapshot = next(&mut reader).await;
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["data"]["instances"]["instances"], json!([]));
        assert_eq!(snapshot["data"]["sessions"]["sessions"], json!([]));
        let mut state = state_stream::StateSnapshot::empty();
        state.instances["instances"] =
            json!([{"name":"review","description":"Message","workspace_only":true}]);
        let publisher = store.clone();
        tokio::task::spawn_blocking(move || publisher.publish(state))
            .await
            .unwrap()
            .unwrap();
        let changed = next(&mut reader).await;
        assert_eq!(changed["type"], "instance.changed");
        assert_eq!(changed["data"]["description"], "Message");
        assert_eq!(changed["version"], 1);
        assert!(
            sequence(changed["cursor"].as_str().unwrap())
                > sequence(snapshot["cursor"].as_str().unwrap())
        );
        server.abort();
        let _ = server.await;
        assert!(!path.exists());
    });
}

#[test]
fn reconnect_after_server_restart_supports_replay_and_fresh_snapshots() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (_home, config, store) = fixture();
        let socket = Listener::bind(&config).unwrap();
        let path = socket_path(&config).unwrap();
        let server = tokio::spawn(serve(store.clone(), socket));
        let mut reader = BufReader::new(UnixStream::connect(&path).await.unwrap());
        let initial = next(&mut reader).await;
        server.abort();
        let _ = server.await;
        let mut state = state_stream::StateSnapshot::empty();
        state.instances["instances"] = json!([{"name":"created-while-down"}]);
        let publisher = store.clone();
        tokio::task::spawn_blocking(move || publisher.publish(state))
            .await
            .unwrap()
            .unwrap();
        let socket = Listener::bind(&config).unwrap();
        let server = tokio::spawn(serve(store, socket));
        let mut stream = UnixStream::connect(&path).await.unwrap();
        stream
            .write_all(format!("{}\n", json!({"since": initial["cursor"]})).as_bytes())
            .await
            .unwrap();
        let mut resumed = BufReader::new(stream);
        assert_eq!(next(&mut resumed).await["type"], "instance.changed");
        assert_eq!(next(&mut resumed).await["type"], "replay.complete");
        let mut fresh = BufReader::new(UnixStream::connect(&path).await.unwrap());
        let snapshot = next(&mut fresh).await;
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(
            snapshot["data"]["instances"]["instances"][0]["name"],
            "created-while-down"
        );
        server.abort();
        let _ = server.await;
    });
}

#[test]
fn an_existing_listener_and_unexpected_path_objects_are_preserved() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (_home, config, _store) = fixture();
        let listener = Listener::bind(&config).unwrap();
        assert!(Listener::bind(&config).is_err());
        let path = socket_path(&config).unwrap();
        drop(listener);
        fs::write(&path, "user-owned file").unwrap();
        assert!(Listener::bind(&config).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "user-owned file");
        fs::remove_file(path).unwrap();
    });
}

#[test]
fn shared_observation_ownership_does_not_scale_with_subscribers() {
    let (_home, config, store) = fixture();
    let lease = store.observation_lease("opencode").unwrap().unwrap();
    let another = Store::new(&config).unwrap();
    assert!(another.observation_lease("opencode").unwrap().is_none());
    store
        .save_observation("opencode", &crate::store::opencode::Snapshot::default())
        .unwrap();
    let (revision, snapshot) = another
        .observation::<crate::store::opencode::Snapshot>("opencode", 0)
        .unwrap()
        .unwrap();
    assert!(snapshot.sessions.is_empty());
    assert!(
        another
            .observation::<crate::store::opencode::Snapshot>("opencode", revision)
            .unwrap()
            .is_none()
    );
    drop(lease);
    assert!(another.observation_lease("opencode").unwrap().is_some());
}

#[test]
fn idle_disconnects_release_subscription_slots_without_waiting_for_events() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (_home, config, store) = fixture();
        let socket = Listener::bind(&config).unwrap();
        let path = socket_path(&config).unwrap();
        let server = tokio::spawn(serve(store, socket));
        for _ in 0..32 {
            let mut reader = BufReader::new(UnixStream::connect(&path).await.unwrap());
            assert_eq!(next(&mut reader).await["type"], "snapshot");
            drop(reader);
        }
        server.abort();
        let _ = server.await;
    });
}

#[test]
fn disconnect_cancels_a_blocked_snapshot_write_immediately() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (_home, _config, store) = fixture();
        let mut snapshot = state_stream::StateSnapshot::empty();
        snapshot.instances["instances"] =
            json!([{"name":"large","description":"x".repeat(4 * 1024 * 1024)}]);
        let publisher = store.clone();
        tokio::task::spawn_blocking(move || publisher.publish(snapshot))
            .await
            .unwrap()
            .unwrap();
        let (unread, stream) = UnixStream::pair().unwrap();
        let (_updates, receiver) = watch::channel(String::new());
        let semaphore = Arc::new(Semaphore::new(1));
        let permit = Arc::new(semaphore.acquire_owned().await.unwrap());
        let task = tokio::spawn(delivery::client(stream, store, receiver, permit));
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(unread);
        let _ = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    });
}
