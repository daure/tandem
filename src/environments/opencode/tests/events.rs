use super::super::events::{Changes, Decoder, LOCAL, Signal};
use super::*;

#[test]
fn native_and_legacy_events_handle_fragmentation_keepalives_and_failure_frames() {
    let mut decoder = Decoder::default();
    assert!(
        !decoder
            .push(b": keepalive\r\n\r\ndata: {\"type\":\"session.ren")
            .unwrap()
    );
    assert!(!decoder.push(b"amed\",\"data\":{\"title\":\"").unwrap());
    assert!(!decoder.push("日本語".as_bytes()).unwrap());
    assert!(decoder.push(b"\"}}\r\n\r\n").unwrap());
    assert!(
        !decoder
            .push(b"data: {\"type\":\"session.message.text.delta\"}\n\n")
            .unwrap()
    );
    assert!(
        decoder
            .push(b"data: {\"payload\":{\"type\":\"session.status\"}}\n\n")
            .unwrap()
    );
    let encoded = json!({"type":"permission.asked"}).to_string();
    assert!(
        decoder
            .push(format!("data: {}\n\n", json!(encoded)).as_bytes())
            .unwrap()
    );
    assert!(
        decoder
            .push(b"event: effect/httpapi/stream/failure\n")
            .is_err()
    );
}

#[test]
fn atomic_receipt_changes_wake_local_observation_without_server_requests() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let signal = Arc::new(Signal::default());
        let mut changes = Changes::new(&observer, Arc::clone(&signal)).unwrap();
        assert!(!changes.sync(&observer, &Snapshot::default()).await.unwrap());
        presence(&observer, "one.tmp", "ses_one", 7, "");
        fs::rename(
            observer.presence.join("one.tmp"),
            observer.presence.join("one.json"),
        )
        .unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), signal.wait())
                .await
                .unwrap(),
            LOCAL
        );
        assert!(changes.sync(&observer, &Snapshot::default()).await.unwrap());
        let snapshot = observer
            .observe_changes(&[], Snapshot::default(), false)
            .await
            .unwrap();
        assert_eq!(snapshot.sessions[0].id, "ses_one");
        assert_eq!(snapshot.sessions[0].panes[0].id, 7);
        assert!(!changes.sync(&observer, &snapshot).await.unwrap());
        fs::remove_file(observer.presence.join("one.json")).unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), signal.wait())
                .await
                .unwrap(),
            LOCAL
        );
        assert!(changes.sync(&observer, &snapshot).await.unwrap());
        let detached = observer
            .observe_changes(&[], snapshot, false)
            .await
            .unwrap();
        assert!(detached.sessions[0].panes.is_empty());
    });
}
