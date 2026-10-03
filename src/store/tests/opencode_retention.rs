use super::*;
use crate::store::opencode::Session;

#[test]
fn unverified_sessions_are_hidden_and_abandoned_after_three_spaced_checks() {
    let mut retention = Retention::default();
    let mut snapshot = Snapshot {
        sessions: vec![Session {
            id: "ses_one".into(),
            stale: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let now = Instant::now();
    assert!(retention.observe_at(&snapshot, now).is_empty());
    assert!(retention.visible_snapshot(&snapshot).sessions.is_empty());
    for _ in 0..100 {
        assert!(retention.observe_at(&snapshot, now).is_empty());
    }
    assert!(retention.exclusions().sessions.is_empty());
    assert!(
        retention
            .observe_at(&snapshot, now + Duration::from_secs(1))
            .is_empty()
    );
    let abandoned = retention.observe_at(&snapshot, now + Duration::from_secs(3));
    assert_eq!(abandoned.len(), 1);
    assert!(retention.exclusions().sessions.contains("ses_one"));
    assert!(retention.next_retry().is_none());
    assert!(
        retention
            .observe_at(&snapshot, now + Duration::from_secs(30))
            .is_empty()
    );

    retention.rediscover();
    assert!(retention.exclusions().sessions.is_empty());
    snapshot.sessions[0].stale = false;
    retention.observe_at(&snapshot, now);
    assert_eq!(retention.visible_snapshot(&snapshot).sessions.len(), 1);
    snapshot.sessions[0].stale = true;
    retention.observe(&snapshot);
    assert!(retention.visible_snapshot(&snapshot).sessions.is_empty());
}

#[test]
fn known_unfinished_missing_workspaces_remain_visible_without_retry_abandonment() {
    let mut snapshot = Snapshot {
        sessions: vec![
            Session {
                id: "ses_work".into(),
                directory: "/work/missing".into(),
                stale: true,
                ..Default::default()
            },
            Session {
                id: "ses_idle".into(),
                directory: "/work/missing".into(),
                activity: crate::store::opencode::Activity::Idle,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    snapshot
        .observation
        .missing_directories
        .insert("/work/missing".into());
    snapshot
        .observation
        .unfinished_sessions
        .insert("ses_work".into());
    let mut retention = Retention::default();
    let now = Instant::now();
    for offset in [0, 1, 3, 30] {
        assert!(
            retention
                .observe_at(&snapshot, now + Duration::from_secs(offset))
                .is_empty()
        );
        assert_eq!(
            retention
                .visible_snapshot(&snapshot)
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["ses_work"]
        );
    }
    assert!(retention.exclusions().sessions.is_empty());
}

#[test]
fn pane_failures_without_cached_rows_have_a_bounded_retry_budget() {
    let mut snapshot = Snapshot::default();
    snapshot
        .observation
        .failures
        .push(super::super::observation::Failure::pane(
            "main",
            7,
            "Pane inventory unavailable",
        ));
    snapshot.error = snapshot.observation_error();
    let mut retention = Retention::default();
    let now = Instant::now();
    assert!(retention.observe_at(&snapshot, now).is_empty());
    assert_eq!(retention.visible_snapshot(&snapshot).error, None);
    retention.rediscover();
    assert!(
        retention
            .observe_at(&snapshot, now + Duration::from_secs(1))
            .is_empty()
    );
    assert_eq!(
        retention
            .observe_at(&snapshot, now + Duration::from_secs(3))
            .len(),
        1
    );
    assert!(retention.exclusions().panes.contains(&("main".into(), 7)));
    assert!(retention.next_retry().is_none());
}
