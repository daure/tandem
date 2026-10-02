use super::*;
use crate::store::providers::{Action, Manifest, Provider, Snapshot, Status};
use std::time::Duration;

fn provider(name: &str, status: Status) -> Provider {
    Provider {
        name: name.into(),
        directory: format!("/templates/providers/{name}"),
        manifest: Some(Manifest {
            schema_version: 1,
            name: format!("dev-{name}"),
            profile: name.into(),
            protocol: "tandem-events-v1".into(),
            description: format!("Sample {name} provider"),
            feedback: vec![],
        }),
        available: true,
        status,
        container_id: matches!(status, Status::Running | Status::Paused | Status::Stopped)
            .then(|| format!("container-{name}")),
        error: None,
        operation: None,
    }
}

#[test]
fn provider_menus_offer_actions_for_the_observed_runtime_state() {
    use crate::app::row_actions::{Command, Target};

    for (status, expected) in [
        (
            Status::NotStarted,
            [true, false, false, false, false, false],
        ),
        (Status::Running, [false, true, true, false, true, true]),
        (Status::Paused, [false, true, false, true, true, true]),
        (Status::Stopped, [true, false, false, false, true, true]),
        (Status::Unknown, [false, false, false, false, false, false]),
    ] {
        let target = Target::Provider(Box::new(provider("message", status)));
        for (command, enabled) in [
            Command::Start,
            Command::Stop,
            Command::Pause,
            Command::Resume,
            Command::Restart,
            Command::Logs,
        ]
        .into_iter()
        .zip(expected)
        {
            assert_eq!(
                target.enabled(command),
                enabled,
                "{status:?}: {}",
                command.label()
            );
        }
    }
    let mut row = provider("message", Status::Stopped);
    row.container_id = None;
    assert!(!Target::Provider(Box::new(row)).enabled(Command::Logs));

    let mut row = provider("message", Status::Stopped);
    row.available = false;
    let target = Target::Provider(Box::new(row));
    assert!(!target.enabled(Command::Start));
    assert!(target.enabled(Command::Logs));

    let mut row = provider("message", Status::Running);
    row.operation = Some(Action::Stop);
    let target = Target::Provider(Box::new(row));
    assert!(!target.enabled(Command::Pause));
    assert!(!target.enabled(Command::Logs));
}

#[test]
fn unavailable_provider_shortcuts_warn_before_confirmation_or_execution() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.pages_mut().update_providers(Snapshot {
        providers: vec![provider("message", Status::NotStarted)],
        error: None,
    });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    app.handle_message(
        Msg::ShowProvider("dev-message".into()),
        &mut EventCtx::default(),
    );
    let (layout, _) = render(&mut app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
        .unwrap();
    for key in ['a', 'r', 'l'] {
        let mut ctx = EventCtx::default();
        app.dispatch_event(
            &EventRoute::new(target.path.clone()),
            &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
            &mut ctx,
        );
        assert_eq!(ctx.messages().len(), 1, "{key}");
        for message in ctx.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
        assert!(app.provider_confirmation.is_none(), "{key}");
        assert!(app.provider_actions.is_empty(), "{key}");
        let notice = app.notifications.center().history().last().unwrap();
        assert_eq!(notice.kind(), tuicore::NotificationKind::Warning);
        assert_eq!(notice.title(), "Provider action unavailable");
        assert!(notice.body().contains("Start"), "{}", notice.body());
    }

    app.handle_message(
        Msg::OpenRowMenu(crate::app::row_actions::Target::Provider(Box::new(
            provider("message", Status::NotStarted),
        ))),
        &mut EventCtx::default(),
    );
    render(&mut app, 130);
    for character in "Stop".chars() {
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
            &mut EventCtx::default(),
        );
    }
    let before = app.notifications.center().history().len();
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::default(),
    );
    assert_eq!(app.notifications.center().history().len(), before + 1);
    assert!(app.provider_confirmation.is_none());
    assert!(app.provider_actions.is_empty());
    assert_eq!(
        app.notifications.center().history().last().unwrap().kind(),
        tuicore::NotificationKind::Warning,
    );
}

#[test]
fn a_running_provider_action_allows_stopping_another_provider() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.pages_mut().update_providers(Snapshot {
        providers: vec![
            provider("message", Status::Running),
            provider("ticket", Status::Running),
        ],
        error: None,
    });
    let (_sender, receiver) = tokio::sync::oneshot::channel();
    app.provider_actions
        .push(crate::app::providers::PendingAction {
            name: "ticket".into(),
            action: Action::Restart,
            receiver,
        });

    app.handle_message(
        Msg::ProviderAction("message".into(), Action::Stop),
        &mut EventCtx::default(),
    );

    assert_eq!(
        app.provider_confirmation,
        Some(("message".into(), Action::Stop))
    );
    assert!(app.view.is_active());
    assert_eq!(app.notifications.center().history().len(), 0);
}

#[test]
fn a_running_provider_action_blocks_another_action_on_the_same_provider() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.pages_mut().update_providers(Snapshot {
        providers: vec![provider("message", Status::Running)],
        error: None,
    });
    let (_sender, receiver) = tokio::sync::oneshot::channel();
    app.provider_actions
        .push(crate::app::providers::PendingAction {
            name: "message".into(),
            action: Action::Restart,
            receiver,
        });

    app.handle_message(
        Msg::ProviderAction("message".into(), Action::Stop),
        &mut EventCtx::default(),
    );

    assert!(app.provider_confirmation.is_none());
    assert!(!app.view.is_active());
    assert_eq!(app.provider_actions.len(), 1);
    let notice = app.notifications.center().history().last().unwrap();
    assert_eq!(notice.title(), "Provider action unavailable");
    assert!(notice.body().contains("this provider"));
}

#[test]
fn concurrent_provider_actions_report_completion_without_losing_pending_results() {
    let mut app = root(AppService::for_tests());
    let (ticket_sender, ticket_receiver) = tokio::sync::oneshot::channel();
    let (message_sender, message_receiver) = tokio::sync::oneshot::channel();
    for (name, action, receiver) in [
        ("ticket", Action::Restart, ticket_receiver),
        ("message", Action::Stop, message_receiver),
    ] {
        app.provider_actions
            .push(crate::app::providers::PendingAction {
                name: name.into(),
                action,
                receiver,
            });
    }

    assert!(!app.poll_provider_action());
    message_sender.send(Ok("message stopped".into())).unwrap();
    assert!(app.poll_provider_action());
    assert_eq!(app.provider_actions.len(), 1);
    assert_eq!(app.provider_actions[0].name, "ticket");
    assert_eq!(
        app.notifications.center().history().last().unwrap().body(),
        "message stopped"
    );

    ticket_sender.send(Ok("ticket restarted".into())).unwrap();
    assert!(app.poll_provider_action());
    assert!(app.provider_actions.is_empty());
    assert_eq!(app.notifications.center().history().len(), 2);
    assert_eq!(
        app.notifications.center().history().last().unwrap().body(),
        "ticket restarted"
    );
    assert!(!app.poll_provider_action());
}

#[test]
fn provider_runtime_preconditions_warn_while_execution_failures_report_errors() {
    use crate::store::providers::ActionError;

    let mut app = root(AppService::for_tests());
    for (error, kind, title) in [
        (
            ActionError::Unavailable("Provider has no collector container; use Start first"),
            tuicore::NotificationKind::Warning,
            "Provider action unavailable",
        ),
        (
            ActionError::Failed("Docker exited with an error".into()),
            tuicore::NotificationKind::Error,
            "Provider action failed",
        ),
    ] {
        let message = error.to_string();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.provider_actions
            .push(crate::app::providers::PendingAction {
                name: "message".into(),
                action: Action::Pause,
                receiver,
            });
        sender.send(Err(error)).unwrap();
        assert!(app.poll_provider_action());
        let notice = app.notifications.center().history().last().unwrap();
        assert_eq!(notice.kind(), kind);
        assert_eq!(notice.title(), title);
        assert_eq!(notice.body(), message);
        assert!(app.provider_actions.is_empty());
    }
}

#[test]
fn provider_shortcuts_choose_start_stop_pause_resume_and_restart_from_runtime_state() {
    init_ui();
    let mut app = root(AppService::for_tests());
    for (status, start_stop, pause_resume) in [
        (Status::NotStarted, Action::Start, Action::Pause),
        (Status::Running, Action::Stop, Action::Pause),
        (Status::Paused, Action::Stop, Action::Resume),
        (Status::Stopped, Action::Start, Action::Pause),
    ] {
        app.pages_mut().update_providers(Snapshot {
            providers: vec![provider("message", status)],
            error: None,
        });
        app.pages_mut()
            .tick(Duration::ZERO, AnimationSettings::default());
        app.handle_message(
            Msg::ShowProvider("dev-message".into()),
            &mut EventCtx::default(),
        );
        let (layout, _) = render(&mut app, 130);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
            .unwrap();
        for (key, expected) in [
            ('s', start_stop),
            ('a', pause_resume),
            ('r', Action::Restart),
        ] {
            let mut ctx = EventCtx::default();
            app.dispatch_event(
                &EventRoute::new(target.path.clone()),
                &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
                &mut ctx,
            );
            assert!(
                matches!(ctx.messages(), [Msg::ProviderAction(name, action)]
                if name == "message" && *action == expected),
                "{status:?}: {key}"
            );
        }
    }
}

#[test]
fn compact_provider_rows_show_full_history_counts_and_refresh_without_runtime_changes() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.pages_mut().update_providers(Snapshot {
        providers: vec![
            provider("message", Status::Running),
            provider("ticket", Status::Stopped),
            provider("system_event", Status::Paused),
            provider("generic", Status::NotStarted),
        ],
        error: None,
    });
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            total: 700,
            provider_totals: [("dev-message".into(), 662), ("dev-ticket".into(), 38)].into(),
            ..Default::default()
        });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    app.handle_message(
        Msg::ShowProvider("dev-message".into()),
        &mut EventCtx::default(),
    );
    let (_, text) = render(&mut app, 130);
    let lines: Vec<_> = text.lines().collect();
    let message = lines
        .iter()
        .position(|line| line.contains(" dev-message ·  0/662 · Running"))
        .unwrap();
    assert!(
        lines[message + 1].contains(" dev-ticket ·  0/38 · Stopped"),
        "{text}"
    );
    assert!(
        lines[message + 2].contains(" dev-system_event ·  0/0 · Paused"),
        "{text}"
    );
    assert!(
        lines[message + 3].contains(" dev-generic ·  0/0 · Not started"),
        "{text}"
    );
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            total: 701,
            provider_totals: [("dev-message".into(), 663), ("dev-ticket".into(), 38)].into(),
            ..Default::default()
        });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    assert!(
        render(&mut app, 130)
            .1
            .contains(" dev-message ·  0/663 · Running")
    );
}

fn render(app: &mut App, width: u16) -> (tuicore::LayoutCtx, String) {
    let area = Rect::new(0, 0, width, 30);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(area, &mut layout);
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            app.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    (layout, rendered_lines(&terminal, area).join("\n"))
}

#[test]
fn event_and_provider_details_share_the_bottom_docked_size_across_terminal_resizes() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let row = rows::from_snapshot(&super::snapshot()).remove(0);
    let event = crate::store::events::Record {
        sequence: 1,
        provider: "dev-message".into(),
        received_at: "now".into(),
        event: crate::environments::events::tests::event("sample"),
        attempts: vec![],
    };
    for kind in ["Event details", "Provider details", "Template details"] {
        match kind {
            "Template details" => app.open_details(&row, &mut EventCtx::default()),
            "Event details" => app.open_event(&event, &mut EventCtx::default()),
            _ => app.open_provider_details(
                &provider("message", Status::Running),
                &mut EventCtx::default(),
            ),
        }
        for width in [130, 40, 80, 130] {
            let (_, text) = render(&mut app, width);
            let lines: Vec<_> = text.lines().collect();
            let header = lines.iter().position(|line| line.contains(kind)).unwrap();
            let panel_width = if width < 100 { width } else { width * 75 / 100 };
            assert_eq!(header, 6, "{text}");
            assert_eq!(lines[header].trim().chars().count(), panel_width as usize);
            if panel_width < width {
                let left = (width - panel_width) / 2;
                let right = left + panel_width - 1;
                assert_eq!(lines[29].chars().nth(left as usize), Some('│'));
                assert_eq!(lines[29].chars().nth(right as usize), Some('│'));
            }
        }
        app.handle_message(Msg::Close, &mut EventCtx::default());
    }
}

#[test]
fn providers_are_the_third_tab_with_owned_lifecycle_controls_and_confirmation() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    app.update_snapshot(super::snapshot());
    app.pages_mut().update_providers(Snapshot {
        providers: vec![
            provider("message", Status::NotStarted),
            provider("ticket", Status::Paused),
        ],
        error: None,
    });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    for _ in 0..2 {
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
            &mut EventCtx::default(),
        );
    }
    assert_eq!(app.tabs_mut().selected_index(), 2);
    assert!(app.providers_active);
    for width in [40, 130] {
        let (layout, text) = render(&mut app, width);
        assert!(
            text.contains("Providers") && text.contains("Paused"),
            "{text}"
        );
        assert!(
            layout
                .focus_targets()
                .iter()
                .any(|target| target.enabled && target.id.as_str() == crate::app::providers::FOCUS)
        );
    }
    let (layout, _) = render(&mut app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
        .unwrap();
    let route = EventRoute::new(target.path.clone());
    let mut menu = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('.'))),
        &mut menu,
    );
    for message in menu.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    assert!(app.menu_layer().is_active());
    let text = render(&mut app, 130).1;
    for (label, key) in [
        ("View details", "Enter"),
        ("Start", "s"),
        ("Stop", "s"),
        ("Pause", "a"),
        ("Resume", "a"),
        ("Restart", "r"),
        ("Logs", "l"),
        ("Events", "e"),
    ] {
        let line = text
            .lines()
            .find(|line| line.contains(&format!("{label}   ")))
            .unwrap();
        assert!(line.trim_end_matches([' ', '┃']).ends_with(key), "{line}");
    }
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Esc)),
        &mut EventCtx::default(),
    );
    assert!(!app.menu_layer().is_active());
    let mut action = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('s'))),
        &mut action,
    );
    assert!(
        matches!(action.messages(), [Msg::ProviderAction(name, Action::Start)] if name == "message")
    );
    app.handle_message(
        Msg::ProviderAction("message".into(), Action::Start),
        &mut EventCtx::default(),
    );
    assert!(app.view.is_active());
    assert!(app.provider_actions.is_empty());
    assert_eq!(
        app.provider_confirmation,
        Some(("message".into(), Action::Start))
    );
    for width in [40, 60, 80, 130] {
        let (layout, text) = render(&mut app, width);
        let dialog = layout.overlays().last().unwrap();
        assert!(dialog.area.width <= width.min(60), "{text}");
        assert_eq!(dialog.area.height, 3, "{text}");
        assert!(text.contains("Start message?"), "{text}");
        assert!(text.contains("Ok (o) · Cancel (c)"), "{text}");
    }
    app.handle_message(Msg::Close, &mut EventCtx::default());
    assert!(app.provider_confirmation.is_none());
    assert!(app.provider_actions.is_empty());
}

#[test]
fn entering_providers_focuses_the_data_view_without_stealing_focus_from_its_controls() {
    init_ui();
    for width in [40, 130] {
        for previous in [1, 3] {
            for (mouse, routed) in [(false, false), (false, true), (true, true)] {
                let mut app = crate::app::root(AppService::for_tests());
                app.pages_mut().update_providers(Snapshot {
                    providers: vec![provider("message", Status::Running)],
                    error: None,
                });
                app.pages_mut()
                    .tick(Duration::ZERO, AnimationSettings::default());
                app.tabs_mut().select_index(previous);
                app.sync_overview_tab(&mut EventCtx::default());
                let (layout, text) = render(&mut app, width);
                let tabs = layout
                    .focus_targets()
                    .iter()
                    .find(|target| target.id.as_str() == "tabs")
                    .unwrap();
                let event = if mouse {
                    let column = text
                        .lines()
                        .next()
                        .unwrap()
                        .split_once("Providers")
                        .unwrap()
                        .0
                        .chars()
                        .count();
                    TuiEvent::Mouse(tuicore::MouseEvent {
                        kind: tuicore::MouseEventKind::Down(tuicore::MouseButton::Left),
                        column: column as u16,
                        row: 0,
                        modifiers: KeyModifiers::NONE,
                    })
                } else {
                    TuiEvent::Key(KeyEvent::from(Key::Char(if previous == 1 {
                        ']'
                    } else {
                        '['
                    })))
                };
                let mut ctx = EventCtx::default();
                if routed {
                    app.dispatch_event(&EventRoute::new(tabs.path.clone()), &event, &mut ctx);
                } else {
                    app.event(&event, &mut ctx);
                }
                assert!(
                    app.providers_active,
                    "{width}: {previous}: {mouse}: {routed}"
                );
                assert!(
                    matches!(ctx.focus_request(), Some(tuicore::FocusRequest::Target(id))
                            if id.as_str() == crate::app::providers::FOCUS),
                    "{width}: {previous}: {mouse}: {routed}: {:?}",
                    ctx.focus_request()
                );
                let (layout, _) = render(&mut app, width);
                let list = layout
                    .focus_targets()
                    .iter()
                    .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
                    .unwrap();
                assert!(list.enabled);
                let mut focus = tuicore::FocusManager::new();
                let transition = focus
                    .apply_request(ctx.focus_request().unwrap(), layout.focus_targets())
                    .unwrap();
                tuicore::TreeDispatcher::new().dispatch_focus(
                    &mut app,
                    transition,
                    AnimationSettings::default(),
                );
                assert_eq!(focus.current().unwrap().id, list.id);
                assert_eq!(focus.current().unwrap().path, list.path);
                app.dispatch_focus(list, false, &mut tuicore::FocusCtx::default());
                let control = layout
                    .focus_targets()
                    .iter()
                    .find(|target| {
                        target
                            .path
                            .keys()
                            .iter()
                            .any(|key| key.as_str() == "provider-pause-all")
                    })
                    .unwrap();
                app.dispatch_focus(control, true, &mut tuicore::FocusCtx::default());
                let mut ctx = EventCtx::default();
                app.dispatch_event(
                    &EventRoute::new(control.path.clone()),
                    &TuiEvent::Key(KeyEvent::from(Key::Left)),
                    &mut ctx,
                );
                assert!(ctx.focus_request().is_none());
            }
        }
    }
}

#[test]
fn provider_data_view_is_focusable_and_tabbable_in_every_runtime_state() {
    init_ui();
    let settings = AnimationSettings {
        enabled: false,
        ..Default::default()
    };
    for width in [40, 130] {
        for status in [
            Status::NotStarted,
            Status::Stopped,
            Status::Running,
            Status::Paused,
        ] {
            let mut app = crate::app::root(AppService::for_tests());
            app.pages_mut().update_providers(Snapshot {
                providers: vec![provider("message", status), provider("ticket", status)],
                error: None,
            });
            app.pages_mut().tick(Duration::ZERO, settings);
            let mut ctx = EventCtx::new(settings);
            app.handle_message(Msg::ShowProvider("dev-message".into()), &mut ctx);
            let mut layout = LayoutEngine::new();
            layout.layout(&mut app, Rect::new(0, 0, width, 30));
            let mut focus = tuicore::FocusManager::new();
            let mut dispatcher = tuicore::TreeDispatcher::new();
            let transition = focus
                .apply_request(ctx.focus_request().unwrap(), layout.focus_targets())
                .unwrap();
            dispatcher.dispatch_focus(&mut app, transition, settings);
            assert_eq!(
                focus.current().unwrap().id.as_str(),
                crate::app::providers::FOCUS,
                "{width}: {status:?}"
            );
            dispatcher.dispatch_event(
                &mut app,
                &EventRoute::new(focus.current_path()),
                &TuiEvent::Key(KeyEvent::from(Key::Down)),
                settings,
            );
            let effects = dispatcher.dispatch_event(
                &mut app,
                &EventRoute::new(focus.current_path()),
                &TuiEvent::Key(KeyEvent::from(Key::Char('s'))),
                settings,
            );
            assert!(
                matches!(effects.messages.as_slice(), [Msg::ProviderAction(name, _)] if name == "ticket")
            );

            let mut visited = Vec::new();
            for _ in 0..layout.focus_targets().len() + 1 {
                let effects = dispatcher.dispatch_event(
                    &mut app,
                    &EventRoute::new(focus.current_path()),
                    &TuiEvent::Key(KeyEvent::from(Key::Tab)),
                    settings,
                );
                if effects.focus_request.is_none() {
                    assert_eq!(effects.outcome, tuicore::EventOutcome::Ignored);
                    assert_eq!(effects.propagation, tuicore::Propagation::Continue);
                }
                let request = effects.focus_request.unwrap_or(tuicore::FocusRequest::Next);
                let transition = focus
                    .apply_request(&request, layout.focus_targets())
                    .unwrap();
                dispatcher.dispatch_focus(&mut app, transition, settings);
                visited.push(focus.current().unwrap().clone());
            }
            let first_control = layout
                .focus_targets()
                .iter()
                .find(|target| {
                    target.enabled
                        && target.id.as_str() == "button"
                        && target
                            .path
                            .keys()
                            .iter()
                            .any(|key| key.as_str().starts_with("provider-"))
                })
                .unwrap();
            assert_eq!(visited[0].path, first_control.path, "{width}: {status:?}");
            assert_eq!(visited[0].id, first_control.id, "{width}: {status:?}");
            assert!(
                visited
                    .iter()
                    .any(|target| target.id.as_str() == crate::app::providers::FOCUS),
                "{width}: {status:?}: {visited:?}"
            );
            for control in layout.focus_targets().iter().filter(|target| {
                target.enabled
                    && target.tab_stop
                    && target.id.as_str() == "button"
                    && target
                        .path
                        .keys()
                        .iter()
                        .any(|key| key.as_str().starts_with("provider-"))
            }) {
                assert!(
                    visited
                        .iter()
                        .any(|target| target.path == control.path && target.id == control.id),
                    "{width}: {status:?}: {control:?}"
                );
                for key in [
                    KeyEvent::from(Key::Esc),
                    KeyEvent {
                        code: Key::Char('['),
                        modifiers: KeyModifiers::CONTROL,
                    },
                ] {
                    let request = tuicore::FocusRequest::TargetAt {
                        path: control.path.clone(),
                        id: control.id.clone(),
                    };
                    if let Some(transition) = focus.apply_request(&request, layout.focus_targets())
                    {
                        dispatcher.dispatch_focus(&mut app, transition, settings);
                    }
                    let effects = dispatcher.dispatch_event(
                        &mut app,
                        &EventRoute::new(focus.current_path()),
                        &TuiEvent::Key(key),
                        settings,
                    );
                    assert_eq!(effects.outcome, tuicore::EventOutcome::Handled);
                    assert!(effects.messages.is_empty());
                    let transition = focus
                        .apply_request(
                            effects.focus_request.as_ref().unwrap(),
                            layout.focus_targets(),
                        )
                        .unwrap();
                    dispatcher.dispatch_focus(&mut app, transition, settings);
                    assert_eq!(
                        focus.current().unwrap().id.as_str(),
                        crate::app::providers::FOCUS
                    );
                }
            }
        }
    }
}

#[test]
fn provider_event_links_apply_an_exact_source_filter_and_restore_source_selection() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    app.update_snapshot(super::snapshot());
    app.pages_mut().update_providers(Snapshot {
        providers: vec![
            provider("message", Status::Running),
            provider("ticket", Status::Running),
        ],
        error: None,
    });
    let rows = ["dev-message", "dev-message-other"]
        .into_iter()
        .enumerate()
        .map(|(index, source)| crate::store::events::Record {
            sequence: index as i64,
            provider: source.into(),
            received_at: "now".into(),
            event: crate::environments::events::tests::event(&format!("event-{index}")),
            attempts: vec![],
        })
        .collect();
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            records: rows,
            total: 2,
            provider_totals: Default::default(),
            error: None,
        });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    app.handle_message(
        Msg::ProviderEvents("dev-message".into()),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let (layout, text) = render(&mut app, 130);
    assert!(text.contains(" dev-message · Alex"), "{text}");
    assert!(!text.contains(" dev-message-other · Alex"), "{text}");
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    let mut action = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(KeyEvent::from(Key::Char('p'))),
        &mut action,
    );
    assert!(matches!(action.messages(), [Msg::ShowProvider(source)] if source == "dev-message"));
    app.handle_message(
        Msg::ShowProvider("dev-message".into()),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    assert!(app.providers_active);
    let (layout, _) = render(&mut app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
        .unwrap();
    let mut action = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(KeyEvent::from(Key::Char('s'))),
        &mut action,
    );
    assert!(
        matches!(action.messages(), [Msg::ProviderAction(name, Action::Stop)] if name == "message")
    );
    let mut menu = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(KeyEvent::from(Key::Char('.'))),
        &mut menu,
    );
    for message in menu.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    render(&mut app, 130);
    for character in "Events".chars() {
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
            &mut EventCtx::default(),
        );
    }
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    assert!(app.events_active);
    assert_eq!(app.tabs_mut().selected_index(), 1);
    let text = render(&mut app, 130).1;
    assert!(text.contains(" dev-message · Alex"), "{text}");
    assert!(!text.contains(" dev-message-other · Alex"), "{text}");
}

#[test]
fn provider_bulk_controls_follow_aggregate_state_and_share_their_slots_and_hotkeys() {
    init_ui();
    let mut app = root(AppService::for_tests());
    for (statuses, start_stop, pause_resume) in [
        (
            [Status::NotStarted, Status::Stopped],
            Action::Start,
            Action::Pause,
        ),
        (
            [Status::Running, Status::Running],
            Action::Stop,
            Action::Pause,
        ),
        (
            [Status::Paused, Status::Paused],
            Action::Stop,
            Action::Resume,
        ),
        (
            [Status::Running, Status::Paused],
            Action::Stop,
            Action::Pause,
        ),
        (
            [Status::Running, Status::Stopped],
            Action::Start,
            Action::Pause,
        ),
        (
            [Status::Paused, Status::Stopped],
            Action::Start,
            Action::Pause,
        ),
    ] {
        let snapshot = Snapshot {
            providers: ["message", "ticket"]
                .into_iter()
                .zip(statuses)
                .map(|(name, status)| provider(name, status))
                .collect(),
            error: None,
        };
        app.pages_mut().update_providers(snapshot.clone());
        app.pages_mut()
            .tick(Duration::ZERO, AnimationSettings::default());
        app.handle_message(
            Msg::ShowProvider("dev-message".into()),
            &mut EventCtx::default(),
        );
        for width in [40, 130] {
            let (layout, text) = render(&mut app, width);
            let toolbar = text.lines().nth(1).unwrap();
            let pause_label = format!("{pause_resume:?}");
            let start_label = format!("{start_stop:?}");
            let pause = toolbar.find(&pause_label).unwrap();
            let start = toolbar.find(&start_label).unwrap();
            let restart = toolbar.find("Restart").unwrap();
            assert!(pause < start && start < restart, "{text}");
            if width == 130 {
                assert!(toolbar.contains(&format!("{pause_label} all")), "{toolbar}");
                assert!(toolbar.contains(&format!("{start_label} all")), "{toolbar}");
                assert!(toolbar.trim_end().ends_with("|R|"), "{toolbar}");
            }
            let list = layout
                .focus_targets()
                .iter()
                .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
                .unwrap();
            for (key, action) in [
                ('A', pause_resume),
                ('S', start_stop),
                ('R', Action::Restart),
            ] {
                let mut ctx = EventCtx::default();
                app.dispatch_event(
                    &EventRoute::new(list.path.clone()),
                    &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
                    &mut ctx,
                );
                if snapshot.action_targets(action).is_empty() {
                    assert!(ctx.messages().is_empty(), "{statuses:?}: {key}");
                } else {
                    assert!(
                        matches!(ctx.messages(), [Msg::ProviderBulkAction(actual)] if *actual == action),
                        "{statuses:?}: {key}: {:?}",
                        ctx.messages()
                    );
                }
            }
            for (slot, action) in [
                ("provider-pause-all", pause_resume),
                ("provider-start-all", start_stop),
                ("provider-restart-all", Action::Restart),
            ] {
                let target = layout
                    .focus_targets()
                    .iter()
                    .find(|target| target.path.keys().iter().any(|key| key.as_str() == slot))
                    .unwrap();
                assert_eq!(target.enabled, !snapshot.action_targets(action).is_empty());
                if !target.enabled {
                    continue;
                }
                app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
                let mut ctx = EventCtx::default();
                app.dispatch_event(
                    &EventRoute::new(target.path.clone()),
                    &TuiEvent::Key(KeyEvent::from(Key::Enter)),
                    &mut ctx,
                );
                assert!(
                    matches!(ctx.messages(), [Msg::ProviderBulkAction(actual)] if *actual == action)
                );
                app.dispatch_focus(target, false, &mut tuicore::FocusCtx::default());
            }
        }
    }
}

#[test]
fn provider_bulk_confirmation_targets_only_eligible_idle_providers_and_cancel_keeps_them_untouched()
{
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut unavailable = provider("missing", Status::Stopped);
    unavailable.available = false;
    let mut busy = provider("busy", Status::Running);
    busy.operation = Some(Action::Stop);
    let snapshot = Snapshot {
        providers: vec![
            provider("message", Status::Running),
            provider("ticket", Status::Paused),
            provider("system_event", Status::Stopped),
            provider("generic", Status::NotStarted),
            provider("unknown", Status::Unknown),
            unavailable,
            busy,
        ],
        error: None,
    };
    app.pages_mut().update_providers(snapshot);
    for (action, expected) in [
        (Action::Start, vec!["system_event", "generic"]),
        (Action::Stop, vec!["message", "ticket"]),
        (Action::Pause, vec!["message"]),
        (Action::Resume, vec!["ticket"]),
        (
            Action::Restart,
            vec!["message", "ticket", "system_event", "missing"],
        ),
    ] {
        app.handle_message(Msg::ProviderBulkAction(action), &mut EventCtx::default());
        let expected: Vec<String> = expected.into_iter().map(str::to_owned).collect();
        assert_eq!(
            app.provider_bulk_confirmation,
            Some((expected.clone(), action))
        );
        assert!(app.provider_actions.is_empty());
        for width in [40, 60, 80, 130] {
            let (layout, text) = render(&mut app, width);
            let dialog = layout.overlays().last().unwrap();
            assert!(dialog.area.width <= width.min(60), "{text}");
            assert_eq!(dialog.area.height, 3, "{text}");
            assert!(
                text.contains(&format!("{action:?} {} providers?", expected.len())),
                "{text}"
            );
            assert!(text.contains("Ok (o) · Cancel (c)"), "{text}");
        }
        app.handle_message(Msg::Close, &mut EventCtx::default());
        assert!(app.provider_bulk_confirmation.is_none());
        assert!(app.provider_actions.is_empty());
    }
    let (_sender, receiver) = tokio::sync::oneshot::channel();
    app.provider_actions
        .push(crate::app::providers::PendingAction {
            name: "message".into(),
            action: Action::Restart,
            receiver,
        });
    app.handle_message(
        Msg::ProviderBulkAction(Action::Pause),
        &mut EventCtx::default(),
    );
    assert!(app.provider_bulk_confirmation.is_none());
    assert_eq!(
        app.notifications.center().history().last().unwrap().kind(),
        tuicore::NotificationKind::Warning
    );
}

#[test]
fn provider_bulk_shortcuts_do_not_run_while_searching_and_empty_controls_are_disabled() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.pages_mut().select_providers();
    let (layout, _) = render(&mut app, 130);
    for slot in [
        "provider-pause-all",
        "provider-start-all",
        "provider-restart-all",
    ] {
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| target.path.keys().iter().any(|key| key.as_str() == slot))
            .unwrap();
        assert!(!target.enabled);
    }
    app.pages_mut().update_providers(Snapshot {
        providers: vec![provider("message", Status::Running)],
        error: None,
    });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let (layout, _) = render(&mut app, 130);
    let list = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::providers::FOCUS)
        .unwrap();
    let route = EventRoute::new(list.path.clone());
    app.dispatch_focus(list, true, &mut tuicore::FocusCtx::default());
    for key in ['/', 'A', 'S', 'R'] {
        let mut ctx = EventCtx::default();
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
            &mut ctx,
        );
        assert!(ctx.messages().is_empty(), "{key}: {:?}", ctx.messages());
    }
}
