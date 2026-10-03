use super::*;
use crate::app::{instances, opencode as projection};
use crate::store::opencode::{Activity, Client, Pane, Session, Snapshot};

pub(super) fn observation() -> Snapshot {
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    Snapshot {
        sessions: [
            ("busy", Activity::Busy, true),
            ("idle", Activity::Idle, true),
            ("detached", Activity::Busy, false),
            ("saved", Activity::Idle, false),
        ]
        .into_iter()
        .map(|(id, activity, attached)| Session {
            id: id.into(),
            title: format!("Conversation {id}"),
            directory: "/tmp/workspaces/review/repo".into(),
            activity,
            panes: if attached {
                vec![
                    pane.clone(),
                    Pane {
                        id: 8,
                        ..pane.clone()
                    },
                ]
            } else {
                Vec::new()
            },
            last_question: Some(format!("Question {id}")),
            question_observed: true,
            activity_elapsed_milliseconds: Some(29_000),
            ..Default::default()
        })
        .collect(),
        clients: vec![Client {
            title: "Empty client".into(),
            directory: "/tmp/workspaces/review".into(),
            server: "http://127.0.0.1:4199".into(),
            pane,
            stale: false,
            awaiting_presence_since: None,
        }],
        error: None,
        ..Default::default()
    }
}

fn render(app: &mut super::super::App, width: u16) -> (tuicore::LayoutCtx, Vec<String>) {
    let area = Rect::new(0, 0, width, 30);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(area, &mut layout);
    let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            app.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    (layout, rendered_lines(&terminal, area))
}

fn click(area: Rect) -> TuiEvent {
    TuiEvent::Mouse(tuicore::MouseEvent {
        kind: tuicore::MouseEventKind::Down(tuicore::MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn sessions_tab_instance_purge_targets_only_the_selected_instance() {
    init_ui();
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(observation());
    let mut app = crate::app::root(service);
    app.update_snapshot(snapshot());
    instances::set_highlighted(&app.instances, Some("instance:review".into()));
    assert!(app.selected().unwrap().parent.is_none());
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('p'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    assert!(matches!(app.intent, Some(crate::app::Intent::Purge(ref name)) if name == "review"));
    let (_, lines) = render(&mut app, 130);
    assert!(lines.iter().any(|line| line.contains("Purge instance")));
    assert!(app.service.operations().is_empty());
}

#[test]
fn app_starts_in_the_expanded_agent_view_with_the_first_item_selected() {
    init_ui();
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(observation());
    let mut app = crate::app::root(service);
    app.update_snapshot(snapshot());

    let (_, lines) = render(&mut app, 130);
    let text = lines.join("\n");
    assert!(app.attached_sessions_only);
    assert!(!app.opencode_history);
    assert!(app.running_only);
    assert_eq!(app.selected().unwrap().id, "instance:review");
    assert!(text.contains("Conversation busy"), "{text}");
    assert!(text.contains("Conversation idle"), "{text}");
}

#[test]
fn agents_view_shows_attached_clients_before_the_first_message() {
    init_ui();
    for (directory, parent, id) in [
        (
            "/tmp/workspaces/review",
            "instance:review",
            "opencode-client:review:main:7",
        ),
        (
            "/tmp/external-new",
            "opencode-workspace:/tmp/external-new",
            "opencode-client:external:/tmp/external-new:main:7",
        ),
    ] {
        let mut app = crate::app::root(AppService::for_tests());
        let mut observation = observation();
        observation.sessions.clear();
        observation.clients[0].directory = directory.into();
        app.service.set_opencode_snapshot_for_tests(observation);
        app.update_snapshot(snapshot());
        for history in [false, true] {
            app.handle_message(
                Msg::SetOpencodeHistory(history),
                &mut EventCtx::new(AnimationSettings::default()),
            );
            let (_, lines) = render(&mut app, 130);
            let text = lines.join("\n");
            assert!(text.contains("Empty client"), "{text}");
            assert!(text.contains("(new session)"), "{text}");
            instances::set_highlighted(&app.instances, Some(id.into()));
            let client = app.selected().unwrap();
            assert_eq!(client.parent.as_deref(), Some(parent));
            assert!(matches!(
                client.opencode,
                Some(projection::Target::Client { .. })
            ));
        }
    }
}

#[test]
fn attached_view_groups_two_line_sessions_under_single_line_instances() {
    init_ui();
    let mut inventory = snapshot();
    inventory.instances[0].description =
        "A deliberately long instance description that should be truncated".into();
    for status in ["healthy", "up", "down (exit 0)", "restarting"] {
        inventory.instances[0].services[0].status = status.into();
        let rows = rows::from_snapshot(&inventory);
        let owner = rows
            .iter()
            .find(|row| row.instance.is_some())
            .unwrap()
            .clone();
        let flat = projection::attached_rows(rows::from_snapshot(&inventory), &observation());
        assert_eq!(flat.len(), 4);
        let group = &flat[0];
        assert_eq!(group.parent, None);
        assert_eq!(group.instance.as_deref(), Some("review"));
        assert_eq!(group.label, owner.label);
        assert_eq!(group.icon, owner.icon);
        assert_eq!(
            group.tone,
            if status == "down (exit 0)" {
                rows::Tone::Success
            } else {
                owner.tone
            }
        );
        assert_eq!(group.loading, owner.loading);
        assert_eq!(group.status, owner.status);
        assert_eq!(group.template_capabilities, "· 󰠲 website");
        assert_eq!(group.status_detail, None);
        assert_eq!(group.hide_resources, owner.hide_resources);
        assert_eq!(group.height(), 1);
        let group_text = group.text("⠋", Some(72));
        assert_eq!(group_text.lines.len(), 1);
        assert_eq!(
            group_text.lines[0].to_string(),
            format!(
                "{} {} · 󰠲 website · {}",
                if owner.loading { "⠋" } else { owner.icon },
                owner.label,
                owner.description
            )
        );
        assert_eq!(
            group_text.lines[0].spans.last().unwrap().style.fg,
            Some(tuicore::theme().muted_fg())
        );
        let separator = &group_text.lines[0].spans[3];
        assert_eq!(separator.content, " · ");
        assert_eq!(separator.style.fg, Some(tuicore::theme().text_fg()));
        let search = group.search_text();
        assert!(search.contains("website"));
        assert!(
            search.contains("A deliberately long instance description that should be truncated")
        );
        for row in &flat[1..] {
            assert_eq!(row.parent.as_deref(), Some("instance:review"));
            assert!(row.opencode.as_ref().unwrap().attached());
            assert_eq!(row.height(), 2);
            assert!(row.hide_resources);
            let text = row.text("⠋", Some(72));
            if matches!(row.opencode, Some(projection::Target::Client { .. })) {
                assert_eq!(text.lines[0].to_string(), "󰚩 Empty client");
                assert_eq!(text.lines[1].to_string(), " (new session)");
                continue;
            }
            assert!(text.lines[0].to_string().starts_with("󰚩 Conversation "));
            assert!(
                text.lines[1].to_string().starts_with("⠋ Question ")
                    || text.lines[1].to_string().starts_with(" Question ")
            );
        }
    }
    inventory.instances[0].description.clear();
    let flat = projection::attached_rows(rows::from_snapshot(&inventory), &observation());
    let text = flat[0].text("⠋", None);
    let description = text.lines[0].spans.last().unwrap();
    assert_eq!(description.content, "(no description)");
    assert_eq!(description.style.fg, Some(tuicore::theme().subtle_fg()));
    let mut unknown = observation();
    unknown.sessions[0].last_question = None;
    unknown.sessions[0].question_observed = false;
    let flat = projection::attached_rows(rows::from_snapshot(&inventory), &unknown);
    let row = flat.iter().find(|row| row.id.ends_with(":busy")).unwrap();
    assert_eq!(row.text("⠋", None).lines.len(), 1);
}

#[test]
fn attached_mode_keeps_history_and_running_filters_enabled() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut inventory = snapshot();
    inventory.instances[0].services[0].status = "down (exit 0)".into();
    app.service.set_opencode_snapshot_for_tests(observation());
    app.update_snapshot(inventory.clone());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    for key in ['O', '[', '[', '[', '['] {
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Char(key))), &mut ctx);
    }
    assert!(app.attached_sessions_only && app.running_only && app.opencode_history);
    for width in [40, 80, 130] {
        let (layout, lines) = render(&mut app, width);
        let text = lines.join("\n");
        assert!(lines[0].contains("Sessions · Events"), "{text}");
        if width >= 80 {
            assert!(lines[0].contains("Providers · Instances"), "{text}");
        }
        let history = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "opencode-history")
            })
            .unwrap();
        assert!(history.enabled);
        let running = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "running-only")
            })
            .unwrap();
        assert!(running.enabled);
        app.dispatch_focus(running, true, &mut tuicore::FocusCtx::default());
        let mut activation = EventCtx::new(AnimationSettings::default());
        app.dispatch_event(
            &EventRoute::new(running.path.clone()),
            &TuiEvent::Key(KeyEvent::from(Key::Char(' '))),
            &mut activation,
        );
        assert!(matches!(
            activation.messages(),
            [Msg::SetRunningOnly(false)]
        ));
        for id in ["busy", "idle", "saved"] {
            let index = lines
                .iter()
                .position(|line| line.contains(&format!("Conversation {id}")))
                .unwrap();
            if id != "saved" {
                assert!(
                    lines[index + 1].contains(&format!("Question {id}")),
                    "{text}"
                );
            }
        }
        assert!(text.contains(" review · Stopped · 󰠲 website"), "{text}");
        if width >= 80 {
            assert!(text.contains("website · (no description)"), "{text}");
        }
        assert!(text.contains("Empty client"), "{text}");
        assert!(!text.contains("Services"), "{text}");
    }
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('O'))), &mut ctx);
    assert!(!app.opencode_history);
    assert!(
        !render(&mut app, 130)
            .1
            .join("\n")
            .contains("Conversation saved")
    );
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('O'))), &mut ctx);
    assert!(app.opencode_history);
    instances::set_highlighted(&app.instances, Some("opencode:review:saved".into()));
    assert_eq!(
        app.selected().unwrap().parent.as_deref(),
        Some("instance:review")
    );
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('A'))), &mut ctx);
    assert!(!app.running_only);
    let filtered = render(&mut app, 130).1.join("\n");
    assert!(filtered.contains("Conversation busy"), "{filtered}");
    assert!(filtered.contains("Conversation saved"), "{filtered}");
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('A'))), &mut ctx);
    assert!(app.running_only);
    assert!(
        render(&mut app, 130)
            .1
            .join("\n")
            .contains("Conversation busy")
    );
    app.handle_message(Msg::SetOpencodeHistory(false), &mut ctx);
    app.handle_message(Msg::SetRunningOnly(true), &mut ctx);
    assert!(app.running_only && !app.opencode_history);
    app.update_snapshot(inventory);
    assert_eq!(app.tabs_mut().selected_index(), 0);
    let (layout, lines) = render(&mut app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == "tabs")
        .unwrap();
    let column = lines[0].split_once("Instances").unwrap().0.chars().count() as u16;
    let mut activation = EventCtx::new(AnimationSettings::default());
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &click(Rect::new(column, 0, 1, 1)),
        &mut activation,
    );
    assert_eq!(app.tabs_mut().selected_index(), 4);
    assert!(!app.attached_sessions_only);
    assert!(app.running_only && !app.opencode_history);
    let (layout, lines) = render(&mut app, 130);
    assert!(lines.join("\n").contains("review"));
    instances::set_highlighted(&app.instances, Some("opencode:review:busy".into()));
    assert_eq!(app.selected().unwrap().id, "opencode:review:busy");
    for name in ["opencode-history", "running-only"] {
        assert!(
            layout.focus_targets().iter().any(|target| target.enabled
                && target.path.keys().iter().any(|key| key.as_str() == name))
        );
    }
}

#[test]
fn filter_toggles_center_the_selected_row() {
    init_ui();
    let settings = AnimationSettings {
        enabled: false,
        ..AnimationSettings::default()
    };
    for (agents, selected_id, label) in [
        (false, "instance:instance-15", "instance-15 ·"),
        (true, "instance:instance-15", "instance-15 ·"),
        (
            true,
            "opencode:instance-15:instance-15-true",
            "Conversation instance-15-true",
        ),
    ] {
        for key in ['O', 'A'] {
            let mut app = root(AppService::for_tests());
            let mut inventory = snapshot();
            let instance = inventory.instances[0].clone();
            inventory.instances = (0..30)
                .map(|index| {
                    let mut instance = instance.clone();
                    instance.name = format!("instance-{index:02}");
                    instance.workspace = format!("/tmp/workspaces/{}", instance.name);
                    if index % 2 == 0 {
                        instance.services[0].status = "down (exit 0)".into();
                    }
                    instance
                })
                .collect();
            let session = observation().sessions[1].clone();
            let sessions = inventory
                .instances
                .iter()
                .flat_map(|instance| {
                    [true, false].map(|attached| {
                        let mut session = session.clone();
                        session.id = format!("{}-{attached}", instance.name);
                        session.title = format!("Conversation {}", session.id);
                        session.directory = instance.workspace.clone();
                        if !attached {
                            session.panes.clear();
                        }
                        session
                    })
                })
                .collect();
            app.service.set_opencode_snapshot_for_tests(Snapshot {
                sessions,
                ..Default::default()
            });
            app.update_snapshot(inventory.clone());
            app.handle_message(
                Msg::SetAttachedSessionsOnly(agents),
                &mut EventCtx::new(settings),
            );
            let (layout, _) = render(&mut app, 130);
            let tree = layout
                .focus_targets()
                .iter()
                .find(|target| target.id.as_str() == super::super::TREE_FOCUS)
                .unwrap();
            app.dispatch_focus(tree, true, &mut tuicore::FocusCtx::default());
            let route = EventRoute::new(tree.path.clone());
            for _ in 0..200 {
                if app.selected().is_some_and(|row| row.id == selected_id) {
                    break;
                }
                app.dispatch_event(
                    &route,
                    &TuiEvent::Key(KeyEvent::from(Key::Down)),
                    &mut EventCtx::new(settings),
                );
            }
            assert_eq!(app.selected().unwrap().id, selected_id);
            for _ in 0..2 {
                app.dispatch_event(
                    &route,
                    &TuiEvent::Mouse(tuicore::MouseEvent {
                        kind: tuicore::MouseEventKind::ScrollDown,
                        column: tree.area.x + 1,
                        row: tree.area.y + 2,
                        modifiers: KeyModifiers::NONE,
                    }),
                    &mut EventCtx::new(settings),
                );
                let before_refresh = render(&mut app, 130).1;
                app.update_snapshot(inventory.clone());
                assert_eq!(render(&mut app, 130).1, before_refresh);
                app.event(
                    &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
                    &mut EventCtx::new(settings),
                );
                let (_, lines) = render(&mut app, 130);
                assert_eq!(app.selected().unwrap().id, selected_id);
                let expected_y = usize::from(tree.area.y + 1 + (tree.area.height - 3) / 2);
                assert!(
                    lines[expected_y].contains(label),
                    "agents={agents}, key={key}, selected={selected_id}, expected row {expected_y}:\n{}",
                    lines.join("\n")
                );
            }
        }
    }
}

#[test]
fn agents_history_controls_conversations_while_preserving_workspace_groups() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut history = observation();
    history.sessions.retain(Session::saved);
    history.clients.clear();
    let mut external = history.sessions[0].clone();
    external.id = "external-saved".into();
    external.directory = "/tmp/external-history".into();
    history.sessions.push(external);
    app.service.set_opencode_snapshot_for_tests(history);
    app.update_snapshot(snapshot());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut ctx);
    app.handle_message(Msg::SetRunningOnly(false), &mut ctx);
    for enabled in [true, false, true] {
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('O'))), &mut ctx);
        assert_eq!(app.opencode_history, enabled);
        for (id, parent) in [
            ("opencode:review:saved", "instance:review"),
            (
                "opencode:external:/tmp/external-history:external-saved",
                "opencode-workspace:/tmp/external-history",
            ),
        ] {
            instances::set_highlighted(&app.instances, Some(id.into()));
            let selected = app.selected();
            assert_eq!(selected.is_some(), enabled, "{id}");
            if let Some(row) = selected {
                assert_eq!(row.parent.as_deref(), Some(parent));
            }
            instances::set_highlighted(&app.instances, Some(parent.into()));
            assert!(app.selected().is_some(), "{parent}");
        }
    }
}

#[test]
fn attached_mode_updates_on_detach_and_leaves_the_view_when_integration_is_disabled() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.service.set_opencode_snapshot_for_tests(observation());
    app.update_snapshot(snapshot());
    app.handle_message(
        Msg::SetAttachedSessionsOnly(true),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    render(&mut app, 130);
    instances::set_highlighted(&app.instances, Some("opencode:review:busy".into()));
    assert!(
        app.selected()
            .unwrap()
            .opencode
            .as_ref()
            .unwrap()
            .attached()
    );
    for index in [1, 5] {
        app.action(index, &mut EventCtx::new(AnimationSettings::default()));
        assert!(app.intent.is_none());
        assert!(!app.view.is_active());
    }
    for index in [3, 6, 7] {
        let mut ctx = EventCtx::new(AnimationSettings::default());
        app.action(index, &mut ctx);
        assert!(match &app.intent {
            Some(crate::app::Intent::Stop(name)) => index == 3 && name == "review",
            Some(crate::app::Intent::Purge(name)) => index == 6 && name == "review",
            Some(crate::app::Intent::Restart { name, service }) => {
                index == 7 && name == "review" && service.is_none()
            }
            _ => false,
        });
        assert!(app.view.is_active());
        app.handle_message(Msg::Close, &mut ctx);
    }
    app.service
        .set_opencode_snapshot_for_tests(Snapshot::default());
    app.update_snapshot(snapshot());
    let text = render(&mut app, 130).1.join("\n");
    assert!(text.contains("No attached OpenCode sessions"), "{text}");
    assert!(!text.contains("Conversation busy"), "{text}");
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(app.service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    app.update_snapshot(snapshot());
    assert!(!app.attached_sessions_only);
    assert_eq!(app.tabs_mut().selected_index(), 0);
    let (_, lines) = render(&mut app, 130);
    assert!(!lines[0].contains("Sessions"));
    assert!(lines[0].contains("Instances"));
    assert!(lines.join("\n").contains("review"));
}
