use super::*;
use crate::store::opencode::Session;

#[test]
fn stale_visibility_uses_elapsed_time_and_recovery_starts_a_fresh_grace_period() {
    let mut retention = Retention::default();
    let mut snapshot = Snapshot {
        sessions: vec![Session {
            id: "ses_one".into(),
            stale: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    for _ in 0..100 {
        retention.observe(&snapshot);
    }
    assert_eq!(retention.visible_snapshot(&snapshot).sessions.len(), 1);
    retention
        .sessions
        .insert("ses_one".into(), Instant::now() - STALE_GRACE);
    assert!(retention.visible_snapshot(&snapshot).sessions.is_empty());
    snapshot.sessions[0].stale = false;
    retention.observe(&snapshot);
    snapshot.sessions[0].stale = true;
    retention.observe(&snapshot);
    assert_eq!(retention.visible_snapshot(&snapshot).sessions.len(), 1);
}
