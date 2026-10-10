use super::*;
use crate::{
    app::{
        instances::{self, Instances},
        opencode as projection,
    },
    store::opencode::{Activity, Client, Pane, Session, SessionLocation, Snapshot},
};

fn pane(id: u32) -> Pane {
    Pane {
        session: "main".into(),
        id,
        tab_id: 4,
        tab_name: "review".into(),
    }
}

fn home() -> Snapshot {
    Snapshot {
        clients: vec![Client {
            title: "OpenCode".into(),
            directory: "/tmp/workspaces/review".into(),
            server: String::new(),
            pane: pane(7),
            stale: false,
            awaiting_presence_since: None,
        }],
        ..Default::default()
    }
}

fn conversation() -> Snapshot {
    Snapshot {
        sessions: vec![Session {
            id: "ses_new".into(),
            title: "New conversation".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Busy,
            panes: vec![pane(7)],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn project(observation: &Snapshot, agents: bool) -> Vec<rows::Row> {
    if agents {
        projection::attached_rows(rows::from_snapshot(&snapshot()), observation)
    } else {
        let mut rows = rows::from_snapshot(&snapshot());
        projection::append_rows(&mut rows, observation, false);
        rows
    }
}

fn select(tree: &mut Instances, state: &instances::SharedState, id: &str) {
    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    for _ in 0..20 {
        if instances::selected(state).is_some_and(|row| row.id == id) {
            return;
        }
        tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('j'))), &mut ctx);
    }
    panic!("Could not select {id}");
}

#[test]
fn closing_a_session_selects_its_original_neighbor_and_failure_does_not_steal_focus() {
    init_ui();
    for agents in [false, true] {
        let mut observation = conversation();
        observation.sessions = ["alpha", "middle", "zulu"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| Session {
                id: format!("ses_{name}"),
                panes: vec![pane(index as u32 + 7)],
                ..observation.sessions[0].clone()
            })
            .collect();
        let mut app = root(AppService::for_tests());
        app.opencode_history = true;
        app.service
            .set_opencode_snapshot_for_tests(observation.clone());
        app.handle_message(
            Msg::SetAttachedSessionsOnly(agents),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        app.update_snapshot(snapshot());
        let state = app.instances.clone();
        let mut tree = Instances::new(state.clone());
        tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
        tree.expand_for_tests("instance:review");
        tree.expand_for_tests("sessions:review");
        select(&mut tree, &state, "opencode:review:ses_middle");
        let closing = app.optimistically_close_opencode_pane("ses_middle", &pane(8));
        tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
        assert_eq!(
            instances::selected(&state).unwrap().id,
            "opencode:review:ses_zulu"
        );
        observation.sessions[1].panes[0].tab_name = "renamed".into();
        app.service.set_opencode_snapshot_for_tests(observation);
        app.update_snapshot(snapshot());
        tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
        assert_eq!(
            instances::selected(&state).unwrap().id,
            "opencode:review:ses_zulu"
        );
        let (sender, reply) = tokio::sync::oneshot::channel();
        sender.send(Err("closure failed".into())).unwrap();
        app.opencode_action = Some(projection::PendingAction::new(
            reply,
            "Cannot close OpenCode session",
            vec![closing],
        ));
        app.poll_opencode_action();
        tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
        assert_eq!(
            instances::selected(&state).unwrap().id,
            "opencode:review:ses_zulu"
        );
        assert!(
            app.opencode_snapshot
                .sessions
                .iter()
                .any(|session| session.id == "ses_middle")
        );
    }
}

#[test]
fn closing_a_session_preserves_sibling_tabs_and_the_home_route_in_its_pane() {
    init_ui();
    let mut observation = home();
    observation.sessions = ["ses_one", "ses_two", "ses_three"]
        .map(|id| Session {
            id: id.into(),
            activity: Activity::Idle,
            ..conversation().sessions[0].clone()
        })
        .to_vec();
    let mut app = root(AppService::for_tests());
    app.service
        .set_opencode_snapshot_for_tests(observation.clone());
    app.update_snapshot(snapshot());
    let closing = app.optimistically_close_opencode_pane("ses_two", &pane(7));
    for _ in 0..2 {
        assert_eq!(
            app.opencode_snapshot
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["ses_one", "ses_three"]
        );
        assert_eq!(app.opencode_snapshot.clients, observation.clients);
        assert!(
            app.opencode_snapshot
                .sessions
                .iter()
                .all(|session| session.panes == [pane(7)])
        );
        app.update_snapshot(snapshot());
    }
    let (sender, reply) = tokio::sync::oneshot::channel();
    sender.send(Err("tab closure refused".into())).unwrap();
    app.opencode_action = Some(projection::PendingAction::new(
        reply,
        "Cannot close OpenCode session",
        vec![closing],
    ));
    app.poll_opencode_action();
    assert_eq!(app.opencode_snapshot, observation);
}

#[test]
fn selection_follows_the_pane_between_home_and_its_first_conversation() {
    init_ui();
    for agents in [false, true] {
        let state = instances::state(project(&home(), agents));
        instances::set_attached_sessions_only(&state, agents);
        let mut tree = Instances::new(state.clone());
        tree.expand_for_tests("instance:review");
        tree.expand_for_tests("sessions:review");
        select(&mut tree, &state, "opencode-client:review:main:7");

        instances::replace_rows(&state, project(&conversation(), agents));
        tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
        assert_eq!(
            instances::selected(&state).unwrap().id,
            "opencode:review:ses_new"
        );

        instances::request_center_highlighted(&state);
        instances::replace_rows(&state, project(&home(), agents));
        tree.layout(Rect::new(0, 0, 120, 40), &mut tuicore::LayoutCtx::new());
        assert_eq!(
            instances::selected(&state).unwrap().id,
            "opencode-client:review:main:7"
        );
    }
}

#[test]
fn selection_follows_the_exact_pane_when_a_conversation_has_multiple_clients() {
    init_ui();
    let state = instances::state(project(&home(), false));
    let mut tree = Instances::new(state.clone());
    tree.expand_for_tests("instance:review");
    tree.expand_for_tests("sessions:review");
    select(&mut tree, &state, "opencode-client:review:main:7");

    let mut observation = conversation();
    observation.sessions[0].panes.insert(0, pane(8));
    observation.sessions[0].panes[1].tab_name = "renamed".into();
    instances::replace_rows(&state, project(&observation, false));
    tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
    assert_eq!(
        instances::selected(&state).unwrap().id,
        "opencode:review:ses_new:main:7"
    );
}

#[test]
fn a_conversation_transition_does_not_steal_selection_from_another_row() {
    init_ui();
    let state = instances::state(project(&home(), false));
    let mut tree = Instances::new(state.clone());
    tree.expand_for_tests("instance:review");
    tree.expand_for_tests("sessions:review");
    select(&mut tree, &state, "services:review");
    instances::replace_rows(&state, project(&conversation(), false));
    tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
    assert_eq!(instances::selected(&state).unwrap().id, "services:review");
}

#[test]
fn a_closed_client_does_not_follow_a_pane_with_the_same_number_in_another_zellij_session() {
    init_ui();
    let state = instances::state(project(&home(), false));
    let mut tree = Instances::new(state.clone());
    tree.expand_for_tests("instance:review");
    tree.expand_for_tests("sessions:review");
    select(&mut tree, &state, "opencode-client:review:main:7");
    let mut observation = conversation();
    observation.sessions[0].panes[0].session = "other".into();
    instances::replace_rows(&state, project(&observation, false));
    tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
    assert_eq!(instances::selected(&state).unwrap().id, "services:review");
}

#[test]
fn creation_selects_the_observed_client_and_follows_its_first_conversation() {
    init_ui();
    for agents in [false, true] {
        for observed_first in [false, true] {
            let mut app = root(AppService::for_tests());
            app.handle_message(
                Msg::SetAttachedSessionsOnly(agents),
                &mut EventCtx::new(AnimationSettings::default()),
            );
            app.handle_message(Msg::SetRunningOnly(false), &mut EventCtx::default());
            let area = Rect::new(0, 0, 120, 40);
            app.update_snapshot(snapshot());
            app.layout(area, &mut tuicore::LayoutCtx::new());
            let (sender, reply) = tokio::sync::oneshot::channel();
            app.opencode_action = Some(projection::PendingAction::creation(
                reply,
                projection::CreationView::Instances(app.instances.clone()),
            ));
            if observed_first {
                app.service.set_opencode_snapshot_for_tests(home());
                app.update_snapshot(snapshot());
                app.layout(area, &mut tuicore::LayoutCtx::new());
            }
            sender
                .send(Ok(SessionLocation {
                    session_id: None,
                    pane: pane(7),
                }))
                .unwrap();
            app.poll_opencode_action();
            app.layout(area, &mut tuicore::LayoutCtx::new());
            if !observed_first {
                let mut unrelated = home();
                unrelated.clients[0].pane.session = "other".into();
                app.service.set_opencode_snapshot_for_tests(unrelated);
                app.update_snapshot(snapshot());
                app.layout(area, &mut tuicore::LayoutCtx::new());
                assert_ne!(
                    app.selected().map(|row| row.id).as_deref(),
                    Some("opencode-client:review:other:7")
                );
                app.service.set_opencode_snapshot_for_tests(home());
                app.update_snapshot(snapshot());
                app.layout(area, &mut tuicore::LayoutCtx::new());
            }
            assert_eq!(app.selected().unwrap().id, "opencode-client:review:main:7");
            app.service.set_opencode_snapshot_for_tests(conversation());
            app.update_snapshot(snapshot());
            app.layout(area, &mut tuicore::LayoutCtx::new());
            assert_eq!(app.selected().unwrap().id, "opencode:review:ses_new");
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal
                .draw(|frame| {
                    let mut ctx = RenderCtx::new();
                    app.render(frame, area, &mut ctx);
                    ctx.flush(frame);
                })
                .unwrap();
            assert!(
                rendered_lines(&terminal, area)
                    .join("\n")
                    .contains("New conversation")
            );
        }
    }
}

#[test]
fn creation_selects_the_exact_session_in_a_shared_client() {
    init_ui();
    for agents in [false, true] {
        for observed_first in [false, true] {
            let mut app = root(AppService::for_tests());
            app.handle_message(
                Msg::SetAttachedSessionsOnly(agents),
                &mut EventCtx::default(),
            );
            let mut observation = conversation();
            observation.sessions = ["ses_first", "ses_last"]
                .map(|id| Session {
                    id: id.into(),
                    ..conversation().sessions[0].clone()
                })
                .to_vec();
            app.service
                .set_opencode_snapshot_for_tests(observation.clone());
            app.update_snapshot(snapshot());
            let area = Rect::new(0, 0, 120, 40);
            app.layout(area, &mut tuicore::LayoutCtx::new());
            let selected = app.selected().unwrap().id;
            let (sender, reply) = tokio::sync::oneshot::channel();
            app.opencode_action = Some(projection::PendingAction::creation(
                reply,
                projection::CreationView::Instances(app.instances.clone()),
            ));
            observation
                .sessions
                .insert(1, conversation().sessions[0].clone());
            if observed_first {
                app.service
                    .set_opencode_snapshot_for_tests(observation.clone());
                app.update_snapshot(snapshot());
                app.layout(area, &mut tuicore::LayoutCtx::new());
            }
            sender
                .send(Ok(SessionLocation {
                    session_id: Some("ses_new".into()),
                    pane: pane(7),
                }))
                .unwrap();
            app.poll_opencode_action();
            app.layout(area, &mut tuicore::LayoutCtx::new());
            if !observed_first {
                assert_eq!(app.selected().unwrap().id, selected);
                app.service.set_opencode_snapshot_for_tests(observation);
                app.update_snapshot(snapshot());
                app.layout(area, &mut tuicore::LayoutCtx::new());
            }
            assert_eq!(app.selected().unwrap().id, "opencode:review:ses_new");
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal
                .draw(|frame| {
                    let mut ctx = RenderCtx::new();
                    app.render(frame, area, &mut ctx);
                    ctx.flush(frame);
                })
                .unwrap();
            assert!(
                rendered_lines(&terminal, area)
                    .join("\n")
                    .contains("New conversation")
            );
        }
    }
}

#[test]
fn created_sessions_select_the_exact_pane_and_expand_its_parents() {
    init_ui();
    let state = instances::state(project(&home(), false));
    let mut tree = Instances::new(state.clone());
    let mut observation = conversation();
    observation.sessions[0].panes.push(pane(8));
    instances::expand_opencode_session(
        &state,
        SessionLocation {
            session_id: Some("ses_new".into()),
            pane: pane(7),
        },
    );
    instances::replace_rows(&state, project(&observation, false));
    tree.layout(Rect::new(0, 0, 120, 40), &mut tuicore::LayoutCtx::new());
    assert_eq!(
        instances::selected(&state).unwrap().id,
        "opencode:review:ses_new:main:7"
    );
    let area = Rect::new(0, 0, 120, 40);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            tree.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    assert!(
        rendered_lines(&terminal, area)
            .join("\n")
            .contains("main / review · pane 7")
    );
}

#[test]
fn created_external_clients_select_their_row_and_expand_their_workspace_group() {
    init_ui();
    for agents in [false, true] {
        let state = instances::state(project(&home(), agents));
        instances::set_attached_sessions_only(&state, agents);
        let mut tree = Instances::new(state.clone());
        let mut observation = home();
        observation.clients[0].directory = "/work/external".into();
        instances::expand_opencode_session(
            &state,
            SessionLocation {
                session_id: None,
                pane: pane(7),
            },
        );
        instances::replace_rows(&state, project(&observation, agents));
        let area = Rect::new(0, 0, 120, 40);
        tree.layout(area, &mut tuicore::LayoutCtx::new());
        assert_eq!(
            instances::selected(&state).unwrap().id,
            "opencode-client:external:/work/external:main:7"
        );
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                tree.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        assert!(
            rendered_lines(&terminal, area)
                .join("\n")
                .contains("(new session)")
        );
    }
}
