use super::*;
use crate::store::opencode::Session;

#[test]
fn ownership_uses_the_nearest_workspace_and_unverified_activity_stays_unknown() {
    let instances = [
        Instance {
            name: "outer".into(),
            workspace: "/work/outer".into(),
            ..Default::default()
        },
        Instance {
            name: "inner".into(),
            workspace: "/work/outer/nested".into(),
            ..Default::default()
        },
    ];
    let snapshot = Snapshot {
        sessions: [
            ("ses_inner", "/work/outer/nested/app", true),
            ("ses_missing", "/work/outer/missing", false),
            ("ses_external", "/work/outer-other", false),
        ]
        .map(|(id, directory, stale)| Session {
            id: id.into(),
            directory: directory.into(),
            activity: Activity::Busy,
            stale,
            ..Default::default()
        })
        .to_vec(),
        observation: super::super::observation::Evidence {
            missing_directories: ["/work/outer/missing".into()].into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let listed = project(snapshot.clone(), &instances, None, true);
    assert_eq!(listed[0].session_id, "ses_external");
    assert_eq!(listed[0].instance, None);
    assert_eq!(listed[1].instance.as_deref(), Some("inner"));
    assert_eq!(listed[1].availability, Availability::Unverified);
    assert_eq!(listed[1].activity, Activity::Unknown);
    assert_eq!(listed[2].availability, Availability::WorkspaceMissing);
    let filtered = project(snapshot, &instances, Some("outer"), true);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].session_id, "ses_missing");
}

#[test]
fn active_listing_includes_open_idle_tabs_and_detached_work_but_history_requires_opt_in() {
    let snapshot = Snapshot {
        sessions: [
            ("ses_open", Activity::Idle, true, false),
            ("ses_running", Activity::Busy, false, false),
            ("ses_waiting", Activity::AwaitingAnswer, false, false),
            ("ses_saved", Activity::Idle, false, false),
            ("ses_unverified", Activity::Busy, false, true),
        ]
        .map(|(id, activity, attached, stale)| Session {
            id: id.into(),
            directory: "/work/app".into(),
            activity,
            panes: if attached {
                vec![Pane {
                    session: "main".into(),
                    id: 1,
                    tab_id: 1,
                    tab_name: "app".into(),
                }]
            } else {
                vec![]
            },
            stale,
            ..Default::default()
        })
        .to_vec(),
        ..Default::default()
    };
    let active = project(snapshot.clone(), &[], None, false);
    assert_eq!(
        active
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        ["ses_open", "ses_running", "ses_waiting"]
    );
    assert_eq!(project(snapshot, &[], None, true).len(), 5);
}
