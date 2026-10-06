use super::*;
use crate::{
    app::{
        Intent,
        acceptances::{Context, Target},
        opencode::SessionAction,
        row_actions::Command,
    },
    store::opencode::{Activity, Pane, Session, Snapshot},
};

fn observation() -> Snapshot {
    Snapshot {
        sessions: vec![Session {
            id: "ses_review".into(),
            title: "Review API".into(),
            directory: "/tmp/workspaces/review".into(),
            server: "http://127.0.0.1:12345".into(),
            activity: Activity::Idle,
            panes: vec![
                Pane {
                    session: "main".into(),
                    id: 7,
                    tab_id: 1,
                    tab_name: "Review".into(),
                },
                Pane {
                    session: "main".into(),
                    id: 8,
                    tab_id: 1,
                    tab_name: "Review".into(),
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn app() -> App {
    let mut app = root(AppService::for_tests());
    app.service.set_opencode_snapshot_for_tests(observation());
    app.update_snapshot(snapshot());
    app
}

#[test]
fn session_and_pane_rows_route_rename_and_delete_before_instance_actions() {
    init_ui();
    for saved in [false, true] {
        for row in [
            "opencode:review:ses_review",
            "opencode:review:ses_review:main:8",
        ] {
            if saved && row.ends_with(":8") {
                continue;
            }
            for key in ['r', 'x'] {
                let mut app = app();
                if saved {
                    let mut observed = observation();
                    observed.sessions[0].panes.clear();
                    app.service.set_opencode_snapshot_for_tests(observed);
                    app.opencode_history = true;
                    app.update_snapshot(snapshot());
                }
                super::super::instances::set_highlighted(&app.instances, Some(row.into()));
                app.event(
                    &TuiEvent::Key(Key::Char(key).into()),
                    &mut EventCtx::default(),
                );
                assert!(
                    match &app.intent {
                        Some(Intent::RenameOpencodeSession(id)) => key == 'r' && id == "ses_review",
                        Some(Intent::DeleteOpencodeSession(id)) => key == 'x' && id == "ses_review",
                        _ => false,
                    },
                    "{row}: {key}"
                );
                assert!(app.service.operations().is_empty());
            }
        }
    }
}

#[test]
fn rename_dialog_has_one_plain_input_and_standard_actions_and_preserves_typed_shortcuts() {
    init_ui();
    let mut app = app();
    app.request_opencode_session_action(
        "ses_review".into(),
        SessionAction::Rename,
        &mut EventCtx::default(),
    );
    let (layout, text) = super::events::render(&mut app, 120);
    assert!(text.contains("Rename OpenCode session"), "{text}");
    assert!(text.contains("Review API"), "{text}");
    assert!(text.contains("Ok") && text.contains("Cancel"), "{text}");
    let focus = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .path
                .keys()
                .last()
                .is_some_and(|key| key.as_str() == "name")
        })
        .unwrap();
    assert_eq!(focus.area.height, 1);
    app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
    let route = EventRoute::new(focus.path.clone());
    let mut ctx = EventCtx::default();
    for key in ['x', 'r', 'o', 'c'] {
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char(key).into()), &mut ctx);
        for message in ctx.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
    }
    assert_eq!(app.name, "Review APIxroc");
    assert!(app.view.is_active());
    assert!(app.opencode_action.is_none());
    app.handle_message(Msg::NameChanged(" ".into()), &mut EventCtx::default());
    app.handle_message(Msg::Submit, &mut EventCtx::default());
    assert!(app.view.is_active());
    assert!(app.opencode_action.is_none());
    app.handle_message(Msg::Close, &mut EventCtx::default());
    assert!(!app.view.is_active());
}

#[test]
fn session_actions_are_available_in_the_menu_and_leave_active_search_as_text() {
    init_ui();
    for key in ['r', 'x'] {
        let mut app = app();
        super::super::instances::set_highlighted(
            &app.instances,
            Some("opencode:review:ses_review".into()),
        );
        app.open_action_menu(&mut EventCtx::default());
        let (layout, text) = super::events::render(&mut app, 120);
        assert!(
            text.contains("Rename OpenCode session") && text.contains("Delete OpenCode session"),
            "{text}"
        );
        let route = EventRoute::new(layout.overlays().last().unwrap().route_path.clone());
        app.dispatch_event(
            &route,
            &TuiEvent::Key(Key::Char(key).into()),
            &mut EventCtx::default(),
        );
        assert!(matches!(app.intent, Some(Intent::RenameOpencodeSession(_))) == (key == 'r'));
        assert!(matches!(app.intent, Some(Intent::DeleteOpencodeSession(_))) == (key == 'x'));
    }
    let mut app = app();
    super::super::instances::set_highlighted(
        &app.instances,
        Some("opencode:review:ses_review".into()),
    );
    super::super::instances::set_searching(&app.instances, true);
    for key in ['r', 'x'] {
        app.event(
            &TuiEvent::Key(Key::Char(key).into()),
            &mut EventCtx::default(),
        );
        assert!(app.intent.is_none());
    }
}

#[test]
fn acceptance_conversations_use_session_actions_and_acceptance_parents_keep_their_actions() {
    init_ui();
    let mut acceptance = super::rules::acceptance(super::rules::rule("inspect"), 71, 7);
    acceptance.instance = "review".into();
    let context = Context {
        inventory: snapshot(),
        opencode: observation(),
    };
    let parent = Target::new(&acceptance, None, &context, true);
    let mut conversation = parent.clone();
    conversation.selected = Some(conversation.conversations[0].clone());
    for (key, action) in [('r', SessionAction::Rename), ('x', SessionAction::Delete)] {
        let mut ctx = EventCtx::default();
        assert!(conversation.action(&TuiEvent::Key(Key::Char(key).into()), &mut ctx));
        assert!(
            matches!(ctx.messages(), [Msg::OpencodeSessionAction(id, found)] if id == "ses_review" && *found == action)
        );
    }
    let target = super::super::row_actions::Target::AcceptanceContext(Box::new(conversation));
    assert!(target.commands().contains(&Command::RenameSession));
    assert!(target.commands().contains(&Command::DeleteSession));
    assert!(target.commands().contains(&Command::ClosePanel));
    assert!(!parent.enabled(Command::ClosePanel));
    for row in &parent.conversations {
        let mut selected = parent.clone();
        selected.selected = Some(row.clone());
        assert!(selected.enabled(Command::ClosePanel));
        let mut ctx = EventCtx::default();
        assert!(selected.action(&TuiEvent::Key(Key::Char('c').into()), &mut ctx));
        assert!(matches!(
            ctx.messages(),
            [Msg::AcceptanceAction(target, Command::ClosePanel)] if target.selected.as_ref().unwrap().id == row.id
        ));
    }
    let mut ctx = EventCtx::default();
    parent.action(&TuiEvent::Key(Key::Char('r').into()), &mut ctx);
    assert!(matches!(
        ctx.messages(),
        [Msg::AcceptanceAction(_, Command::Rule)]
    ));
}

#[test]
fn grey_external_folders_offer_clear_data_on_instances_and_sessions_with_a_compact_dialog() {
    init_ui();
    for sessions in [false, true] {
        for menu in [false, true] {
            let mut app = app();
            app.service.set_opencode_snapshot_for_tests(Snapshot {
                directories: vec!["/work/unused".into()],
                sessions: vec![Session {
                    id: "ses_saved".into(),
                    directory: "/work/unused".into(),
                    activity: Activity::Idle,
                    ..Default::default()
                }],
                ..Default::default()
            });
            app.running_only = false;
            app.opencode_history = true;
            app.select_overview(sessions);
            app.update_snapshot(snapshot());
            super::super::instances::set_highlighted(
                &app.instances,
                Some("opencode-workspace:/work/unused".into()),
            );
            let selected = app.selected().unwrap();
            assert_eq!(
                selected.id, "opencode-workspace:/work/unused",
                "menu={menu}"
            );
            assert!(crate::app::opencode::clearable_folder(&selected).is_some());
            assert_eq!(app.attached_sessions_only, sessions);
            if menu {
                app.open_action_menu(&mut EventCtx::default());
                let (layout, text) = super::events::render(&mut app, 120);
                assert!(text.contains("Clear OpenCode data"), "{text}");
                let route = EventRoute::new(layout.overlays().last().unwrap().route_path.clone());
                app.dispatch_event(
                    &route,
                    &TuiEvent::Key(Key::Char('x').into()),
                    &mut EventCtx::default(),
                );
            } else {
                app.event(
                    &TuiEvent::Key(Key::Char('x').into()),
                    &mut EventCtx::default(),
                );
            }
            assert!(
                matches!(&app.intent, Some(Intent::ClearOpencodeFolder(directory)) if directory == "/work/unused"),
                "menu={menu}"
            );
            let (_, text) = super::events::render(&mut app, 120);
            assert!(
                text.contains("Folder files stay untouched")
                    && text.contains("Ok")
                    && text.contains("Cancel"),
                "{text}"
            );
        }
    }
}

#[test]
fn accepted_folder_cleanup_hides_the_row_without_locking_session_actions() {
    init_ui();
    let mut app = app();
    crate::environments::runtime_db::prepare(&app.service.config_for_tests()).unwrap();
    let server = crate::environments::opencode::tests::history_server::Server::start();
    server.session("ses_saved", "/work/unused", None);
    let observed = Snapshot {
        directories: vec!["/work/unused".into()],
        sessions: vec![Session {
            id: "ses_saved".into(),
            directory: "/work/unused".into(),
            server: server.url.clone(),
            activity: Activity::Idle,
            ..Default::default()
        }],
        ..Default::default()
    };
    app.service.set_opencode_snapshot_for_tests(observed);
    app.running_only = false;
    app.select_overview(true);
    app.update_snapshot(snapshot());
    let mut data = server.data.lock().unwrap();
    data.delete_failure = true;
    let mut ctx = EventCtx::default();
    app.request_clear_opencode_folder("/work/unused".into(), &mut ctx);
    app.handle_message(Msg::Submit, &mut ctx);
    assert_eq!(app.notifications.center().history().len(), 1);
    let notice = app.notifications.center().history().last().unwrap();
    assert_eq!(notice.title(), "OpenCode cleanup");
    assert_eq!(
        notice.body(),
        "Background cleanup task started for /work/unused. Folder files stay untouched."
    );
    assert!(!app.view.is_active());
    assert!(app.opencode_action.is_none());
    assert_eq!(app.opencode_cleanups.len(), 1);
    assert!(
        !app.project_rows(&app.snapshot, &[])
            .iter()
            .any(|row| row.id == "opencode-workspace:/work/unused")
    );
    drop(data);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !app.opencode_cleanups.is_empty() && std::time::Instant::now() < deadline {
        app.poll_opencode_cleanups();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(app.opencode_cleanups.is_empty());
    assert_eq!(app.notifications.center().history().len(), 1);
    app.update_snapshot(snapshot());
    assert!(
        app.project_rows(&app.snapshot, &[])
            .iter()
            .any(|row| row.id == "opencode-workspace:/work/unused")
    );
}

#[test]
fn folder_cleanup_keeps_one_start_notification_through_completion() {
    init_ui();
    for succeeds in [false, true] {
        let mut app = app();
        app.notify(tuicore::Notification::info(
            "OpenCode cleanup",
            "Background cleanup task started for /work/unused. Folder files stay untouched.",
        ));
        let (sender, reply) = tokio::sync::oneshot::channel();
        app.opencode_cleanups.push(reply);
        app.poll_opencode_cleanups();
        assert_eq!(app.notifications.center().history().len(), 1);
        sender
            .send(if succeeds {
                Ok(())
            } else {
                Err("Cleanup failed".into())
            })
            .unwrap();
        app.poll_opencode_cleanups();
        app.poll_opencode_cleanups();
        assert_eq!(app.notifications.center().history().len(), 1);
        let notice = app.notifications.center().history().last().unwrap();
        assert_eq!(notice.kind(), tuicore::NotificationKind::Info);
        assert_eq!(notice.title(), "OpenCode cleanup");
        assert_eq!(
            notice.body(),
            "Background cleanup task started for /work/unused. Folder files stay untouched."
        );
    }
}
