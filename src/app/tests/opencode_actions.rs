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
fn n_creates_in_the_workspace_of_instance_subtrees_sessions_and_clients() {
    tuicore::init();
    let mut inventory = snapshot();
    inventory.instances[0].services.push(InstanceService {
        name: "migrate".into(),
        status: "exited 0".into(),
        one_shot: true,
        ..Default::default()
    });
    for (id, directory) in [
        ("instance:review", "/tmp/workspaces/review"),
        ("setup:review", "/tmp/workspaces/review"),
        ("service:review:migrate", "/tmp/workspaces/review"),
        ("services:review", "/tmp/workspaces/review"),
        ("service:review:web", "/tmp/workspaces/review"),
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
        app.update_snapshot(inventory.clone());
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
fn close_all_hides_busy_saved_and_empty_clients_immediately_and_restores_failures() {
    tuicore::init();
    let mut observation = observation();
    observation.sessions[1].activity = Activity::Busy;
    let mut app = root(AppService::for_tests());
    app.opencode_history = true;
    app.service
        .set_opencode_snapshot_for_tests(observation.clone());
    app.update_snapshot(snapshot());
    app.submit_close_opencode_scope(
        CloseScope::Directory("/work/external".into()),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    for _ in 0..2 {
        assert!(app.opencode_action.is_some());
        assert_eq!(app.opencode_snapshot.sessions.len(), 1);
        assert_eq!(app.opencode_snapshot.sessions[0].id, "ses_review");
        assert!(app.opencode_snapshot.clients.is_empty());
        app.update_snapshot(snapshot());
    }
    let (sender, reply) = tokio::sync::oneshot::channel();
    sender.send(Err("closure failed".into())).unwrap();
    app.opencode_action = Some(super::super::opencode::PendingAction::new(
        reply,
        "Cannot close OpenCode sessions",
        app.closing_opencode_panes.clone(),
    ));
    app.poll_opencode_action();
    assert_eq!(app.opencode_snapshot, observation);
    assert!(
        app.notifications
            .center()
            .history()
            .last()
            .unwrap()
            .body()
            .contains("closure failed")
    );
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
