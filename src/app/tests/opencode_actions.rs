use super::*;
use crate::store::opencode::{Activity, Client, CloseScope, Pane, Session, Snapshot};

fn observation() -> Snapshot {
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    Snapshot {
        sessions: vec![
            Session {
                id: "ses_review".into(),
                directory: "/tmp/workspaces/review".into(),
                activity: Activity::Idle,
                panes: vec![pane.clone()],
                ..Default::default()
            },
            Session {
                id: "ses_external".into(),
                directory: "/work/external".into(),
                activity: Activity::Idle,
                panes: vec![
                    Pane {
                        id: 8,
                        ..pane.clone()
                    },
                    Pane {
                        id: 9,
                        ..pane.clone()
                    },
                ],
                ..Default::default()
            },
        ],
        clients: vec![Client {
            title: "OpenCode".into(),
            directory: "/work/external".into(),
            server: String::new(),
            pane: Pane { id: 10, ..pane },
            stale: false,
            awaiting_presence_since: None,
        }],
        ..Default::default()
    }
}

#[test]
fn n_creates_in_the_workspace_of_instances_groups_sessions_and_clients() {
    tuicore::init();
    for (id, directory) in [
        ("instance:review", "/tmp/workspaces/review"),
        ("sessions:review", "/tmp/workspaces/review"),
        ("opencode:review:ses_review", "/tmp/workspaces/review"),
        ("opencode-workspace:/work/external", "/work/external"),
        (
            "opencode:external:/work/external:ses_external",
            "/work/external",
        ),
        (
            "opencode:external:/work/external:ses_external:main:9",
            "/work/external",
        ),
        (
            "opencode-client:external:/work/external:main:10",
            "/work/external",
        ),
    ] {
        let mut app = root(AppService::for_tests());
        app.service.set_opencode_snapshot_for_tests(observation());
        app.update_snapshot(snapshot());
        super::super::instances::set_highlighted(&app.instances, Some(id.into()));
        assert_eq!(
            super::super::opencode::new_session_directory(&app.selected().unwrap()),
            Some(directory),
            "{id}"
        );
        let mut ctx = EventCtx::new(AnimationSettings::default());
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('n'))), &mut ctx);
        assert!(ctx.notifications().is_empty(), "{id}");
        assert!(app.opencode_action.is_some(), "{id}");
        assert!(!app.view.is_active(), "{id}");
    }
}

#[test]
fn external_aggregate_has_no_creation_directory_and_parents_confirm_bulk_close() {
    tuicore::init();
    for (id, scope) in [
        ("opencode-workspaces", CloseScope::ExternalWorkspaces),
        (
            "opencode-workspace:/work/external",
            CloseScope::Directory("/work/external".into()),
        ),
        ("sessions:review", CloseScope::Instance("review".into())),
        ("instance:review", CloseScope::Instance("review".into())),
    ] {
        let mut app = root(AppService::for_tests());
        app.service.set_opencode_snapshot_for_tests(observation());
        app.update_snapshot(snapshot());
        super::super::instances::set_highlighted(&app.instances, Some(id.into()));
        let mut ctx = EventCtx::new(AnimationSettings::default());
        if id == "opencode-workspaces" {
            app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('n'))), &mut ctx);
            assert!(app.opencode_action.is_none());
            assert!(!app.view.is_active());
        }
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('c'))), &mut ctx);
        assert!(
            matches!(app.intent, Some(super::super::Intent::CloseOpencodeSessions(ref actual)) if *actual == scope),
            "{id}"
        );
        assert!(app.view.is_active());
        assert!(app.opencode_action.is_none());
    }
}
