use super::*;
use crate::store::opencode::Snapshot;

#[test]
fn deletion_markers_filter_shared_observations_and_stale_writes_across_restarts() {
    let (_home, config, store) = fixture();
    let session = Session {
        id: "ses_test".into(),
        server: "http://127.0.0.1:4199".into(),
        directory: "/work/review".into(),
        ..Default::default()
    };
    let mut other = session.clone();
    other.server = "http://127.0.0.1:4299".into();
    let stale = Snapshot {
        sessions: vec![session.clone(), other.clone()],
        ..Default::default()
    };
    store.save_opencode_observation(&stale).unwrap();
    store
        .associate_origin(&session, "review", Some("slack://message/1"))
        .unwrap();
    let cursor = store.boundary(None).unwrap().cursor;
    let revision = store
        .observation::<Snapshot>("opencode", 0)
        .unwrap()
        .unwrap()
        .0;
    store
        .apply_session_edits(&BTreeMap::from([((session.server, session.id), None)]))
        .unwrap();
    let (_, observed) = store
        .observation::<Snapshot>("opencode", revision)
        .unwrap()
        .unwrap();
    assert_eq!(observed.sessions, [other.clone()]);
    assert!(store.retained_sessions().unwrap().is_empty());
    let replay = store.boundary(Some(&cursor)).unwrap();
    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.events[0]["type"], "session.removed");
    store.save_opencode_observation(&stale).unwrap();
    drop(store);
    let restarted = Store::new(&config).unwrap();
    let (_, observed) = restarted
        .observation::<Snapshot>("opencode", 0)
        .unwrap()
        .unwrap();
    assert_eq!(observed.sessions, [other]);
    restarted.publish(snapshot()).unwrap();
    assert_eq!(
        restarted.boundary(None).unwrap().snapshot.sessions["sessions"],
        json!([])
    );
}
