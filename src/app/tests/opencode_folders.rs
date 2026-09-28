use super::*;
use crate::{
    app::{instances, opencode as projection},
    store::opencode::{Activity, Client, Pane, Session, Snapshot},
};

fn observation() -> Snapshot {
    let pane = |id| Pane {
        session: "main".into(),
        id,
        tab_id: 4,
        tab_name: "work".into(),
    };
    Snapshot {
        directories: vec!["/work/b-empty".into(), "/work/a-idle".into()],
        sessions: [
            (
                "owned",
                "/tmp/workspaces/review/repo",
                Activity::Idle,
                false,
            ),
            ("idle", "/work/a-idle", Activity::Idle, true),
            ("history", "/work/c-history", Activity::Idle, false),
            ("detached", "/work/d-detached", Activity::Busy, false),
            ("busy", "/work/z-busy", Activity::Busy, true),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (id, directory, activity, attached))| Session {
            id: id.into(),
            title: id.into(),
            directory: directory.into(),
            activity,
            panes: attached.then(|| pane(index as u32)).into_iter().collect(),
            ..Default::default()
        })
        .collect(),
        clients: vec![Client {
            title: "OpenCode".into(),
            directory: "/work/y-new".into(),
            server: String::new(),
            pane: pane(20),
            stale: false,
            awaiting_presence_since: None,
        }],
        ..Default::default()
    }
}

fn project(observation: &Snapshot, history: bool) -> Vec<rows::Row> {
    let mut inventory = snapshot();
    inventory.instances[0].services[0].status = "down (exit 0)".into();
    let owners = inventory
        .instances
        .iter()
        .map(|instance| crate::store::opencode::resources::Owner {
            name: instance.name.clone(),
            workspace: instance.workspace.clone(),
            template_directory: instance.template_directory.clone(),
        })
        .collect::<Vec<_>>();
    projection::attached_rows_for_owners(
        rows::from_snapshot(&inventory),
        observation,
        &owners,
        history,
        false,
    )
}

fn roots(rows: &[rows::Row]) -> Vec<&str> {
    rows.iter()
        .filter(|row| row.parent.is_none())
        .map(|row| row.id.as_str())
        .collect()
}

#[test]
fn agents_keep_all_known_folders_and_put_open_clients_first_independently_of_history() {
    tuicore::init();
    for history in [false, true] {
        let rows = project(&observation(), history);
        assert_eq!(
            roots(&rows),
            [
                "opencode-workspace:/work/a-idle",
                "opencode-workspace:/work/y-new",
                "opencode-workspace:/work/z-busy",
                "instance:review",
                "opencode-workspace:/work/b-empty",
                "opencode-workspace:/work/c-history",
                "opencode-workspace:/work/d-detached",
            ]
        );
        for id in ["owned", "history", "detached"] {
            assert_eq!(rows.iter().any(|row| matches!(&row.opencode, Some(projection::Target::Session { id: candidate, .. }) if candidate == id)), history);
        }
        for row in &rows {
            if let Some(parent) = &row.parent {
                assert!(rows.iter().any(|candidate| candidate.id == *parent));
            }
        }
    }
}

#[test]
fn folder_activity_uses_clients_outside_the_conversation_display_window() {
    tuicore::init();
    let mut observation = observation();
    observation.sessions.extend((1..=21).map(|index| Session {
        id: format!("saved-{index}"),
        directory: "/work/z-busy".into(),
        activity: Activity::Idle,
        updated: index,
        ..Default::default()
    }));
    let rows = project(&observation, true);
    assert!(
        !rows
            .iter()
            .any(|row| row.id == "opencode:external:/work/z-busy:busy")
    );
    assert_eq!(roots(&rows)[2], "opencode-workspace:/work/z-busy");

    let mut app = root(AppService::for_tests());
    app.service
        .set_opencode_snapshot_for_tests(observation.clone());
    app.update_snapshot(snapshot());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut ctx);
    app.handle_message(Msg::SetOpencodeHistory(true), &mut ctx);
    app.handle_message(Msg::SetRunningOnly(true), &mut ctx);
    assert!(
        roots(&app.project_rows(&snapshot(), &[])).contains(&"opencode-workspace:/work/z-busy")
    );

    for session in &mut observation.sessions {
        session.stale = true;
    }
    for client in &mut observation.clients {
        client.stale = true;
    }
    app.service.set_opencode_snapshot_for_tests(observation);
    app.update_snapshot(snapshot());
    assert_eq!(
        roots(&app.project_rows(&snapshot(), &[])),
        ["instance:review"]
    );
}

#[test]
fn closing_a_folders_last_client_moves_it_below_active_folders_without_losing_selection() {
    tuicore::init();
    let mut observation = observation();
    let state = instances::state(project(&observation, false));
    instances::set_attached_sessions_only(&state, true);
    let mut tree = instances::Instances::new(state.clone());
    let selected = "opencode-workspace:/work/a-idle";
    assert_eq!(instances::selected(&state).unwrap().id, selected);
    observation
        .sessions
        .iter_mut()
        .find(|session| session.id == "idle")
        .unwrap()
        .panes
        .clear();
    let rows = project(&observation, false);
    assert_eq!(
        roots(&rows),
        [
            "opencode-workspace:/work/y-new",
            "opencode-workspace:/work/z-busy",
            "instance:review",
            "opencode-workspace:/work/a-idle",
            "opencode-workspace:/work/b-empty",
            "opencode-workspace:/work/c-history",
            "opencode-workspace:/work/d-detached",
        ]
    );
    instances::replace_rows(&state, rows);
    tree.layout(Rect::new(0, 0, 120, 40), &mut tuicore::LayoutCtx::new());
    assert_eq!(instances::selected(&state).unwrap().id, selected);
    observation.sessions[0].panes.push(Pane {
        session: "main".into(),
        id: 30,
        tab_id: 1,
        tab_name: "review".into(),
    });
    assert_eq!(roots(&project(&observation, false))[2], "instance:review");
}

#[test]
fn empty_known_external_folders_can_launch_a_new_session() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.service.set_opencode_snapshot_for_tests(observation());
    app.update_snapshot(snapshot());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut ctx);
    app.handle_message(Msg::SetRunningOnly(false), &mut ctx);
    instances::set_highlighted(
        &app.instances,
        Some("opencode-workspace:/work/b-empty".into()),
    );
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('n'))), &mut ctx);
    assert!(app.opencode_action.is_some());
    assert!(ctx.notifications().is_empty());
}

#[test]
fn sessions_show_clientless_active_instances_only_with_show_all_in_group_order() {
    tuicore::init();
    let mut observation = observation();
    observation.sessions.retain(Session::attached);
    let mut owned = observation.sessions[0].clone();
    owned.id = "owned".into();
    owned.directory = "/tmp/workspaces/with-session/repo".into();
    observation.sessions.push(owned);
    let mut app = root(AppService::for_tests());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut ctx);
    for (known, starting) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut observation = observation.clone();
        let mut inventory = snapshot();
        inventory.instances[0].pending = starting;
        let mut attached = inventory.instances[0].clone();
        attached.name = "with-session".into();
        attached.workspace = "/tmp/workspaces/with-session".into();
        inventory.instances.push(attached);
        if known {
            observation
                .directories
                .push(inventory.instances[0].workspace.clone());
        }
        app.service
            .set_opencode_snapshot_for_tests(observation.clone());
        app.update_snapshot(inventory.clone());
        for history in [false, true] {
            app.handle_message(Msg::SetOpencodeHistory(history), &mut ctx);
            for _ in 0..2 {
                app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('A'))), &mut ctx);
                let rows = app.project_rows(&inventory, &[]);
                let mut expected = vec![
                    "opencode-workspace:/work/a-idle",
                    "opencode-workspace:/work/y-new",
                    "opencode-workspace:/work/z-busy",
                    "instance:with-session",
                ];
                if !app.running_only {
                    expected.extend(["instance:review", "opencode-workspace:/work/b-empty"]);
                }
                assert_eq!(
                    roots(&rows),
                    expected,
                    "known={known}, starting={starting}, history={history}"
                );
                assert!(
                    !rows
                        .iter()
                        .any(|row| row.parent.as_deref() == Some("instance:review"))
                );
                instances::set_highlighted(&app.instances, Some("instance:review".into()));
                assert_eq!(app.selected().is_some(), !app.running_only);
            }
        }
        inventory.instances[0].pending = false;
        inventory.instances[0].services[0].status = "down (exit 0)".into();
        app.update_snapshot(inventory.clone());
        assert!(!roots(&app.project_rows(&inventory, &[])).contains(&"instance:review"));
    }
}

#[test]
fn running_filter_keeps_live_external_folders_and_preserves_workspace_ownership() {
    tuicore::init();
    let mut inventory = snapshot();
    inventory.instances[0].services[0].status = "down (exit 0)".into();
    let mut app = root(AppService::for_tests());
    app.service.set_opencode_snapshot_for_tests(observation());
    app.update_snapshot(inventory.clone());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut ctx);
    for history in [false, true] {
        app.handle_message(Msg::SetOpencodeHistory(history), &mut ctx);
        app.handle_message(Msg::SetRunningOnly(true), &mut ctx);
        let rows = app.project_rows(&inventory, &[]);
        assert!(rows.iter().all(|row| row.instance.is_none()));
        let mut expected = vec![
            "opencode-workspace:/work/a-idle",
            "opencode-workspace:/work/y-new",
            "opencode-workspace:/work/z-busy",
        ];
        if history {
            expected.push("opencode-workspace:/work/d-detached");
        }
        assert_eq!(roots(&rows), expected);
        for row in &rows {
            if let Some(parent) = &row.parent {
                assert!(rows.iter().any(|candidate| candidate.id == *parent));
            }
        }
        app.handle_message(Msg::SetRunningOnly(false), &mut ctx);
        assert_eq!(
            roots(&app.project_rows(&inventory, &[])),
            roots(&project(&observation(), history))
        );
    }
}
