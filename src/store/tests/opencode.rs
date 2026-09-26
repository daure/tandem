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
    ];
    assert_eq!(
        Counts::of(sessions.iter()),
        Counts {
            live: 3,
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
            "saved"
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
