use super::*;

#[test]
fn counts_overlap_without_counting_multiple_panes_twice() {
    let pane = Pane {
        session: "main".into(),
        id: 1,
        tab_id: 2,
        tab_name: "review".into(),
    };
    let sessions = [
        Session {
            activity: Activity::Busy,
            panes: vec![pane.clone(), pane.clone()],
            ..Default::default()
        },
        Session {
            activity: Activity::Idle,
            panes: vec![pane],
            ..Default::default()
        },
        Session {
            activity: Activity::Busy,
            ..Default::default()
        },
        Session {
            activity: Activity::Idle,
            ..Default::default()
        },
        Session {
            activity: Activity::AwaitingAnswer,
            ..Default::default()
        },
    ];
    assert_eq!(
        Counts::of(sessions.iter()),
        Counts {
            live: 4,
            attached: 2,
            busy: 2
        }
    );
    assert_eq!(
        sessions.map(|session| session.label()),
        [
            "attached · busy",
            "attached · idle",
            "detached · busy",
            "saved",
            "detached · awaiting answer"
        ]
    );
}

#[test]
fn workspace_matching_uses_path_boundaries_and_the_closest_parent() {
    let roots = [("a", "/work/a"), ("nested", "/work/a/nested")];
    assert_eq!(workspace_owner("/work/abc", roots.into_iter()), None);
    assert_eq!(workspace_owner("/work/a/../other", roots.into_iter()), None);
    assert_eq!(
        workspace_owner("/work/a/repo/src", roots.into_iter()),
        Some("a")
    );
    assert_eq!(
        workspace_owner("/work/a/nested/repo", roots.into_iter()),
        Some("nested")
    );
}

#[test]
fn completion_requires_a_fresh_busy_to_ready_transition_for_the_same_session() {
    let snapshot = |id: &str, activity, stale| Snapshot {
        sessions: vec![Session {
            id: id.into(),
            activity,
            stale,
            ..Default::default()
        }],
        ..Default::default()
    };
    let busy = snapshot("session", Activity::Busy, false);

    assert!(!snapshot("session", Activity::Unknown, false).completed_since(&busy));
    for activity in [Activity::Idle, Activity::AwaitingAnswer] {
        let ready = snapshot("session", activity, false);
        assert!(ready.completed_since(&busy));
        assert!(!snapshot("other", activity, false).completed_since(&busy));
        assert!(!snapshot("session", activity, true).completed_since(&busy));
        assert!(!ready.completed_since(&snapshot("session", Activity::Busy, true)));
        assert!(!ready.completed_since(&Snapshot::default()));
        assert!(!ready.completed_since(&ready));
        assert!(!ready.completed_since(&snapshot("session", Activity::AwaitingAnswer, false)));
    }
}
