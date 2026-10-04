use super::*;

fn toolbar_terminal(app: &mut super::super::App, width: u16) -> Terminal<TestBackend> {
    let area = Rect::new(0, 0, width, 30);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            app.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    terminal
}

fn toolbar_line(app: &mut super::super::App, width: u16) -> String {
    rendered_lines(&toolbar_terminal(app, width), Rect::new(0, 0, width, 30))[1].clone()
}

#[test]
fn tabs_share_toolbar_controls_and_retain_separate_searches() {
    init_ui();
    for width in [40, 80, 130] {
        let service = AppService::for_tests();
        service.set_opencode_snapshot_for_tests(super::attached_sessions::observation());
        let mut app = crate::app::root(service);
        app.update_snapshot(snapshot());
        let settings = AnimationSettings {
            enabled: false,
            ..Default::default()
        };
        let area = Rect::new(0, 0, width, 30);
        let mut layout = LayoutEngine::new();
        layout.layout(&mut app, area);
        let tabs = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == "tabs")
            .unwrap()
            .clone();
        let route = EventRoute::new(tabs.path.clone());

        for sessions in [false, true] {
            app.handle_message(
                Msg::SetAttachedSessionsOnly(sessions),
                &mut EventCtx::new(settings),
            );
            assert_eq!(app.attached_sessions_only, sessions);
            let text = rendered_lines(&toolbar_terminal(&mut app, width), area).join("\n");
            assert_eq!(text.contains("Conversation busy"), sessions, "{text}");
            assert_eq!(text.contains("Service"), !sessions, "{text}");
        }

        let mut ctx = EventCtx::new(settings);
        app.handle_message(Msg::SetRunningOnly(true), &mut ctx);
        app.handle_message(Msg::SetOpencodeHistory(true), &mut ctx);
        app.handle_message(Msg::SetCompletionSound(true), &mut ctx);
        layout.layout(&mut app, area);
        let tree = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == super::super::TREE_FOCUS)
            .unwrap()
            .clone();
        app.dispatch_focus(&tree, true, &mut tuicore::FocusCtx::default());
        let tree_route = EventRoute::new(tree.path);
        for key in "/missing-workspace"
            .chars()
            .map(Key::Char)
            .chain([Key::Enter])
        {
            app.dispatch_event(
                &tree_route,
                &TuiEvent::Key(KeyEvent::from(key)),
                &mut EventCtx::new(settings),
            );
        }
        for (label, sessions) in [("Instances", false), ("Sessions", true)] {
            let lines = rendered_lines(&toolbar_terminal(&mut app, width), area);
            if let Some((prefix, _)) = lines[0].split_once(label) {
                let column = prefix.chars().count() as u16;
                app.dispatch_event(
                    &route,
                    &TuiEvent::Mouse(tuicore::MouseEvent {
                        kind: tuicore::MouseEventKind::Down(tuicore::MouseButton::Left),
                        column,
                        row: 0,
                        modifiers: KeyModifiers::NONE,
                    }),
                    &mut EventCtx::new(settings),
                );
            } else {
                app.handle_message(
                    Msg::SetAttachedSessionsOnly(sessions),
                    &mut EventCtx::new(settings),
                );
            }
            assert_eq!(app.attached_sessions_only, sessions);
            assert!(app.running_only && app.opencode_history && app.completion_sound);
            let lines = rendered_lines(&toolbar_terminal(&mut app, width), area);
            assert_eq!(lines[1].contains("󰕾"), sessions);
            if !sessions {
                app.event(
                    &TuiEvent::Key(KeyEvent::from(Key::Char('N'))),
                    &mut EventCtx::new(settings),
                );
                assert!(app.completion_sound);
            }
            assert_eq!(
                lines[2].contains("missing-workspace"),
                sessions,
                "{}",
                lines[2]
            );
            assert_eq!(
                lines[3..].iter().any(|line| line.contains("review")),
                !sessions
            );
            layout.layout(&mut app, area);
            for action in ["stop-all", "purge-all"] {
                assert!(layout.focus_targets().iter().any(|target| target.enabled
                    && target.path.keys().iter().any(|key| key.as_str() == action)));
            }
        }
    }
}

#[test]
fn disabled_opencode_keeps_navigation_on_instances_and_restores_sessions_when_enabled() {
    init_ui();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let settings = AnimationSettings {
        enabled: false,
        ..Default::default()
    };
    for disabled_at_startup in [true, false] {
        let service = AppService::for_tests();
        if disabled_at_startup {
            runtime
                .block_on(service.set_opencode_enabled(false).unwrap())
                .unwrap()
                .unwrap();
        }
        let mut app = crate::app::root(service);
        app.update_snapshot(snapshot());
        if !disabled_at_startup {
            app.handle_message(
                Msg::SetOpencodeIntegration(false),
                &mut EventCtx::new(settings),
            );
            runtime
                .block_on(app.settings_save.take().unwrap())
                .unwrap()
                .unwrap();
            app.update_snapshot(snapshot());
        }
        for width in [40, 130] {
            let area = Rect::new(0, 0, width, 30);
            let mut layout = LayoutEngine::new();
            layout.layout(&mut app, area);
            let tabs = layout
                .focus_targets()
                .iter()
                .find(|target| target.id.as_str() == "tabs")
                .unwrap()
                .clone();
            let route = EventRoute::new(tabs.path);
            let header = rendered_lines(&toolbar_terminal(&mut app, width), area)[0].clone();
            assert!(header.contains("Instances"), "{header}");
            assert!(!header.contains("Sessions"), "{header}");
            for key in ['[', ']'] {
                app.dispatch_event(
                    &route,
                    &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
                    &mut EventCtx::new(settings),
                );
                assert!(!app.attached_sessions_only);
                assert_eq!(
                    app.tabs_mut().selected_index(),
                    if key == '[' { 3 } else { 0 }
                );
                assert_eq!(app.providers_active, key == '[');
            }
            app.handle_message(
                Msg::SetAttachedSessionsOnly(true),
                &mut EventCtx::new(settings),
            );
            assert!(!app.attached_sessions_only);
            app.handle_message(Msg::SetRunningOnly(false), &mut EventCtx::new(settings));
            let overview = layout
                .focus_targets()
                .iter()
                .find(|target| target.hotkey_sequences.iter().any(|key| key == "shift+h"))
                .unwrap();
            app.dispatch_event(
                &EventRoute::new(overview.path.clone()),
                &TuiEvent::Hotkey(HotkeyEvent::Commit("shift+h".into())),
                &mut EventCtx::new(settings),
            );
            assert!(!app.attached_sessions_only);
            assert!(app.running_only);
            assert_eq!(app.tabs_mut().selected_index(), 0);
            let text = rendered_lines(&toolbar_terminal(&mut app, width), area).join("\n");
            assert!(text.contains("review"), "{text}");
            assert!(text.contains("Service"), "{text}");
        }
        runtime
            .block_on(app.service.set_opencode_enabled(true).unwrap())
            .unwrap()
            .unwrap();
        assert!(app.update_snapshot(snapshot()));
        assert!(!app.attached_sessions_only);
        assert_eq!(app.tabs_mut().selected_index(), 2);
        let header =
            rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30))[0].clone();
        assert!(
            header.contains("Sessions · Events · Instances · Rules · Providers"),
            "{header}"
        );
        for _ in 0..2 {
            app.event(
                &TuiEvent::Key(KeyEvent::from(Key::Char('['))),
                &mut EventCtx::new(settings),
            );
        }
        assert!(app.attached_sessions_only);
        assert_eq!(app.tabs_mut().selected_index(), 0);
    }
}

#[test]
fn bracket_navigation_keeps_focus_valid_and_the_tab_header_active() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    app.update_snapshot(snapshot());
    let settings = AnimationSettings {
        enabled: false,
        ..Default::default()
    };
    let mut layout = LayoutEngine::new();
    layout.layout(&mut app, Rect::new(0, 0, 130, 30));
    let targets = layout.focus_targets().to_vec();
    let header = |app: &mut App| {
        let terminal = toolbar_terminal(app, 130);
        (0..130)
            .map(|x| terminal.backend().buffer().cell((x, 0)).unwrap().clone())
            .collect::<Vec<_>>()
    };
    let active_header = header(&mut app);
    let tabs = targets
        .iter()
        .find(|target| target.id.as_str() == "tabs")
        .unwrap();
    app.dispatch_focus(tabs, false, &mut tuicore::FocusCtx::new(settings));
    assert_eq!(header(&mut app), active_header);

    for target in targets.iter().filter(|target| target.enabled) {
        let mut current = target.clone();
        app.dispatch_focus(target, true, &mut tuicore::FocusCtx::new(settings));
        for key in ['[', ']', '[', ']'] {
            let mut ctx = EventCtx::new(settings);
            let outcome = app.dispatch_event(
                &EventRoute::new(current.path.clone()),
                &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
                &mut ctx,
            );
            assert_eq!(outcome, tuicore::EventOutcome::Handled);
            assert_eq!(app.providers_active, key == '[', "{:?}", target.path);
            assert_eq!(
                app.tabs_mut().selected_index(),
                if key == '[' { 4 } else { 0 }
            );
            layout.layout(&mut app, Rect::new(0, 0, 130, 30));
            let next = layout
                .focus_targets()
                .iter()
                .find(|candidate| {
                    candidate.enabled
                        && match ctx.focus_request() {
                            Some(tuicore::FocusRequest::Target(id)) => candidate.id == *id,
                            Some(tuicore::FocusRequest::Path(path)) => {
                                candidate.path == *path && candidate.id == current.id
                            }
                            None => candidate.path == current.path && candidate.id == current.id,
                            request => panic!("Unexpected focus request: {request:?}"),
                        }
                })
                .unwrap();
            app.dispatch_focus(&current, false, &mut tuicore::FocusCtx::new(settings));
            app.dispatch_focus(next, true, &mut tuicore::FocusCtx::new(settings));
            current = next.clone();
        }
        app.dispatch_focus(&current, false, &mut tuicore::FocusCtx::new(settings));
        assert_eq!(header(&mut app), active_header);
    }
    for key in ['[', ']'] {
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
            &mut EventCtx::new(settings),
        );
        assert_eq!(app.providers_active, key == '[');
    }
    let tree = targets
        .iter()
        .find(|target| target.id.as_str() == super::super::TREE_FOCUS)
        .unwrap();
    app.dispatch_focus(tree, true, &mut tuicore::FocusCtx::new(settings));
    for key in "/12][".chars() {
        app.dispatch_event(
            &EventRoute::new(tree.path.clone()),
            &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
            &mut EventCtx::new(settings),
        );
        assert_eq!(app.events_active, key == ']');
    }
    let lines = rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30));
    assert!(lines[2].contains("12"), "{}", lines[2]);
    app.action(2, &mut EventCtx::new(settings));
    assert!(app.view.is_active());
    for key in ['[', ']'] {
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
            &mut EventCtx::new(settings),
        );
        assert!(app.attached_sessions_only);
    }
}

#[test]
fn visibility_toggle_starts_off_and_reveals_all_instances() {
    init_ui();
    let mut inventory = snapshot();
    let mut other = inventory.instances[0].clone();
    other.name = "other".into();
    inventory.instances.push(other);
    let mut stopped = inventory.instances[0].clone();
    stopped.name = "stopped".into();
    stopped.services[0].status = "down (exit 0)".into();
    inventory.instances.push(stopped);
    let mut workspace_template = inventory.templates[0].clone();
    workspace_template.name = "guidance-only".into();
    workspace_template.directory = "/tmp/templates/guidance-only".into();
    workspace_template.compose_file.clear();
    workspace_template.compose_source.clear();
    inventory.templates.push(workspace_template);
    let mut workspace = inventory.instances[0].clone();
    workspace.name = "workspace".into();
    workspace.template = "guidance-only".into();
    workspace.template_directory = "/tmp/templates/guidance-only".into();
    workspace.workspace = "/tmp/workspaces/workspace".into();
    workspace.services.clear();
    workspace.workspace_only = true;
    workspace.runtime.workspace_ready = true;
    inventory.instances.push(workspace);
    let mut agent_template = inventory.templates[0].clone();
    agent_template.name = "agent-only".into();
    agent_template.directory = "/tmp/templates/agent-only".into();
    agent_template.compose_file.clear();
    agent_template.compose_source.clear();
    inventory.templates.push(agent_template);
    let mut agent = inventory.instances[0].clone();
    agent.name = "agent".into();
    agent.template = "agent-only".into();
    agent.template_directory = "/tmp/templates/agent-only".into();
    agent.workspace = "/tmp/workspaces/agent".into();
    agent.services.clear();
    agent.workspace_only = true;
    agent.runtime.workspace_ready = true;
    inventory.instances.push(agent);
    let mut empty_template = inventory.templates[0].clone();
    empty_template.name = "new".into();
    empty_template.directory = "/tmp/templates/new".into();
    inventory.templates.push(empty_template);
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(crate::store::opencode::Snapshot {
        sessions: vec![crate::store::opencode::Session {
            id: "ses_agent".into(),
            title: "Agent session".into(),
            directory: "/tmp/workspaces/agent".into(),
            activity: crate::store::opencode::Activity::Busy,
            ..Default::default()
        }],
        ..Default::default()
    });
    let mut app = root(service);
    app.update_snapshot(inventory.clone());

    for width in [40, 130] {
        let area = Rect::new(0, 0, width, 30);
        let mut layout = tuicore::LayoutCtx::new();
        app.layout(area, &mut layout);
        let target = |name: &str| {
            layout
                .focus_targets()
                .iter()
                .find(|target| target.path.keys().iter().any(|key| key.as_str() == name))
                .unwrap()
        };
        let history = target("opencode-history");
        let running = target("running-only");
        assert_eq!(running.area.right() + 1, history.area.x);
        let line = toolbar_line(&mut app, width);
        assert!(line.contains("󰈈") && line.contains("|A|"), "{line}");
        assert!(line.find("󰈈").unwrap() < line.find("󰋚").unwrap(), "{line}");
    }

    assert!(app.running_only);
    let running =
        rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30)).join("\n");
    assert!(running.contains("review"), "{running}");
    assert!(running.contains("other"), "{running}");
    assert!(running.contains("agent-only"), "{running}");
    assert!(!running.contains("stopped"), "{running}");
    assert!(running.contains("guidance-only"), "{running}");
    assert!(!running.contains("new"), "{running}");
    assert!(running.contains(" 2/3"), "{running}");

    let mut toggle = EventCtx::new(AnimationSettings::default());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('A'))), &mut toggle);
    assert!(toggle.messages().is_empty());
    assert!(!app.running_only);
    let all = rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30)).join("\n");
    assert!(
        all.contains("review")
            && all.contains("other")
            && all.contains("stopped")
            && all.contains("guidance-only")
            && all.contains("agent-only")
            && all.contains("new"),
        "{all}"
    );
    assert!(all.contains(" 2/3"), "{all}");

    inventory
        .instances
        .iter_mut()
        .find(|instance| instance.name == "stopped")
        .unwrap()
        .services[0]
        .status = "up".into();
    app.update_snapshot(inventory.clone());
    let all = rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30)).join("\n");
    assert!(all.contains(" 3/3"), "{all}");

    let mut toggle = EventCtx::new(AnimationSettings::default());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('A'))), &mut toggle);
    assert!(toggle.messages().is_empty());
    assert!(app.running_only);
    inventory
        .instances
        .iter_mut()
        .find(|instance| instance.name == "stopped")
        .unwrap()
        .services[0]
        .status = "down (exit 0)".into();
    app.update_snapshot(inventory);
    let running =
        rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30)).join("\n");
    assert!(running.contains(" 2/3"), "{running}");
    assert!(!running.contains("stopped"), "{running}");
}

#[test]
fn global_h_opens_the_expanded_agent_view_and_restores_default_filters() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.service
        .set_opencode_snapshot_for_tests(super::attached_sessions::observation());
    app.update_snapshot(snapshot());
    let settings = AnimationSettings::default();
    let mut ctx = EventCtx::new(settings);
    app.handle_message(Msg::SetRunningOnly(false), &mut ctx);
    app.handle_message(Msg::SetOpencodeHistory(true), &mut ctx);
    let area = Rect::new(0, 0, 130, 30);
    let mut layout = LayoutEngine::new();
    layout.layout(&mut app, area);
    let overview = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .hotkey_sequences
                .iter()
                .any(|sequence| sequence == "shift+h")
        })
        .unwrap()
        .clone();

    app.dispatch_event(
        &EventRoute::new(overview.path),
        &TuiEvent::Hotkey(HotkeyEvent::Commit("shift+h".into())),
        &mut EventCtx::new(settings),
    );

    assert!(app.running_only);
    assert!(!app.opencode_history);
    assert!(app.attached_sessions_only);
    assert_eq!(app.selected().unwrap().id, "instance:review");
    let terminal = toolbar_terminal(&mut app, 130);
    let lines = rendered_lines(&terminal, Rect::new(0, 0, 130, 30));
    assert!(
        lines[1].contains("○── 󰈈 |A| ○── 󰋚 |O| ○── 󰕾 |N|"),
        "{}",
        lines[1]
    );
    assert_eq!(app.tabs_mut().selected_index(), 0);
    let overview = lines.join("\n");
    assert!(overview.contains("review"), "{overview}");
    assert!(overview.contains("Conversation busy"), "{overview}");
    assert!(overview.contains("Conversation idle"), "{overview}");
    assert!(!overview.contains("Service"), "{overview}");
}

#[test]
fn toolbar_totals_cover_all_instances_and_update_independently_of_tree_search() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut inventory = snapshot();
    inventory.resource_revision = Some(1);
    inventory.available_memory_bytes = Some(8 * 1073741824);
    inventory.cpu_temperature_millicelsius = Some(65_000);
    inventory.instances[0].services[0].usage = Some(crate::store::environments::ResourceUsage {
        memory_bytes: 500 * 1048576,
        cpu_basis_points: Some(25_000),
        sampled_at_unix_seconds: 42,
    });
    let mut second = inventory.instances[0].clone();
    second.name = "other".into();
    second.template = "missing".into();
    second.template_directory = "/tmp/templates/missing".into();
    inventory.instances.push(second);
    app.update_snapshot(inventory.clone());

    for width in [90, 130] {
        let line = toolbar_line(&mut app, width);
        assert!(
            line.trim_end().ends_with("65°C ·  8.0 GiB · 1.0 GiB 500%"),
            "{line}"
        );
        for label in ["used", "available", "CPU"] {
            assert!(!line.contains(label), "{line}");
        }
    }

    let mut layout = tuicore::LayoutCtx::new();
    app.layout(Rect::new(0, 0, 130, 30), &mut layout);
    let tree = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == super::super::TREE_FOCUS)
        .unwrap()
        .clone();
    app.dispatch_focus(&tree, true, &mut tuicore::FocusCtx::default());
    for key in ['/', 'z', 'z'] {
        app.dispatch_event(
            &tuicore::EventRoute::new(tree.path.clone()),
            &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
            &mut EventCtx::new(AnimationSettings::default()),
        );
    }
    assert!(toolbar_line(&mut app, 130).contains("65°C ·  8.0 GiB · 1.0 GiB 500%"));

    inventory.instances.pop();
    inventory.cpu_temperature_millicelsius = Some(66_600);
    inventory.available_memory_bytes = Some(7 * 1073741824);
    app.update_snapshot(inventory.clone());
    assert!(toolbar_line(&mut app, 130).contains("65°C ·  8.0 GiB · 1.0 GiB 500%"));
    inventory.resource_revision = Some(2);
    app.update_snapshot(inventory);
    assert!(
        app.view
            .tick(std::time::Duration::ZERO, AnimationSettings::default())
            .layout
    );
    assert!(toolbar_line(&mut app, 130).contains("67°C ·  7.0 GiB · 0.5 GiB 250%"));
}

#[test]
fn toolbar_totals_align_with_resource_columns_when_the_scrollbar_appears_and_disappears() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut inventory = snapshot();
    inventory.cpu_temperature_millicelsius = Some(65_000);
    inventory.instances[0].services[0].usage = Some(crate::store::environments::ResourceUsage {
        memory_bytes: 500 * 1048576,
        cpu_basis_points: Some(25_000),
        sampled_at_unix_seconds: 42,
    });
    for index in 1..4 {
        let mut instance = inventory.instances[0].clone();
        instance.name = format!("review-{index}");
        inventory.instances.push(instance);
    }
    app.update_snapshot(inventory.clone());

    for (height, padding) in [(30, 1), (12, 2), (30, 1), (12, 2), (12, 1)] {
        if height == 12 && padding == 1 {
            inventory.instances.truncate(1);
            app.update_snapshot(inventory.clone());
        }
        let area = Rect::new(0, 0, 130, height);
        app.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                app.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let percent_x = |y| (0..area.width).find(|&x| buffer.cell((x, y)).unwrap().symbol() == "%");
        let totals_x = percent_x(1).expect("toolbar CPU total");
        assert_eq!(totals_x, area.right() - padding - 1);
        let row_positions: Vec<_> = (3..height - 1).filter_map(percent_x).collect();
        assert!(!row_positions.is_empty());
        assert!(row_positions.iter().all(|&x| x == totals_x));
    }
}

#[test]
fn memory_units_align_with_totals_across_cpu_digit_boundaries() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut inventory = snapshot();
    inventory.available_memory_bytes = Some(8 * 1073741824);
    let mut template = inventory.templates[0].clone();
    template.name = "other".into();
    template.directory = "/tmp/templates/other".into();
    let mut instance = inventory.instances[0].clone();
    instance.name = "other".into();
    instance.template = template.name.clone();
    instance.template_directory = template.directory.clone();
    inventory.templates.push(template);
    inventory.instances.push(instance);

    for cpu in [4950, 5000, 5250, 50_000, 4950] {
        for instance in &mut inventory.instances {
            instance.services[0].usage = Some(crate::store::environments::ResourceUsage {
                memory_bytes: 180 * 1048576,
                cpu_basis_points: Some(cpu),
                sampled_at_unix_seconds: 42,
            });
        }
        app.update_snapshot(inventory.clone());
        for width in [90, 130] {
            let terminal = toolbar_terminal(&mut app, width);
            let lines = rendered_lines(&terminal, Rect::new(0, 0, width, 30));
            let total = &lines[1];
            let memory_x = total[..total.rfind('B').unwrap()].chars().count();
            let cpu_x = total[..total.rfind('%').unwrap()].chars().count();
            let rows = lines[3..]
                .iter()
                .filter(|line| line.contains("0.2 GiB"))
                .collect::<Vec<_>>();
            assert!(!rows.is_empty(), "{lines:#?}");
            for row in rows {
                assert_eq!(
                    row[..row.rfind('B').unwrap()].chars().count(),
                    memory_x,
                    "memory alignment at {cpu} basis points: {lines:#?}"
                );
                assert_eq!(row[..row.rfind('%').unwrap()].chars().count(), cpu_x);
            }
        }
    }
}

#[test]
fn toolbar_totals_show_unavailable_and_paused_states() {
    init_ui();
    let mut app = root(AppService::for_tests());
    app.update_snapshot(Default::default());
    let line = toolbar_line(&mut app, 130);
    assert!(line.trim_end().ends_with(" —"), "{line}");

    let mut guidance_only = snapshot();
    guidance_only.instances[0].services.clear();
    guidance_only.instances[0].workspace_only = true;
    guidance_only.instances[0].runtime.workspace_ready = true;
    app.update_snapshot(guidance_only);
    let line = toolbar_line(&mut app, 130);
    assert!(line.trim_end().ends_with(" —"), "{line}");

    let spinner = tuicore::Spinner::new().glyph().to_owned();
    let mut initial = snapshot();
    initial.available_memory_bytes = Some(20 * 1073741824);
    initial.cpu_temperature_millicelsius = Some(65_000);
    app.update_snapshot(initial);
    let line = toolbar_line(&mut app, 130);
    assert!(line.contains(&spinner), "{line}");
    assert!(
        line.trim_end().ends_with(&format!("65°C · {spinner}")),
        "{line}"
    );
    assert!(!line.contains("GiB") && !line.contains(''), "{line}");

    let mut inventory = snapshot();
    inventory.instances[0].services[0].usage = Some(crate::store::environments::ResourceUsage {
        memory_bytes: 20 * 1048576,
        cpu_basis_points: Some(50_000),
        sampled_at_unix_seconds: 42,
    });
    let mut starting = inventory.instances[0].clone();
    starting.name = "starting".into();
    starting.pending = true;
    inventory.instances.push(starting);
    app.update_snapshot(inventory.clone());
    let line = toolbar_line(&mut app, 130);
    assert!(
        line.contains(&format!(" — {spinner} 20 MiB 500%")),
        "{line}"
    );
    let terminal = toolbar_terminal(&mut app, 130);
    let x = line.chars().position(|character| character == '2').unwrap() as u16;
    assert_eq!(
        terminal.backend().buffer().cell((x, 1)).unwrap().fg,
        tuicore::theme().muted_fg()
    );
    let narrow = toolbar_line(&mut app, 40);
    assert!(narrow.contains("󰈈") && narrow.contains("󰋚"), "{narrow}");
    assert!(
        !narrow.contains("MiB"),
        "unavailable totals should remain hidden when they do not fit: {narrow}"
    );

    inventory.instances.pop();
    app.update_snapshot(inventory.clone());
    let ready = toolbar_line(&mut app, 130);
    assert!(ready.contains(" — · 20 MiB 500%"), "{ready}");
    for value in ["", "20 MiB", "500%"] {
        assert_eq!(
            line[..line.find(value).unwrap()].chars().count(),
            ready[..ready.find(value).unwrap()].chars().count(),
            "{value} must stay aligned while loading changes"
        );
    }
    inventory.instances[0].services[0]
        .usage
        .as_mut()
        .unwrap()
        .cpu_basis_points = None;
    app.update_snapshot(inventory.clone());
    let line = toolbar_line(&mut app, 130);
    assert!(line.trim_end().ends_with(&spinner), "{line}");
    assert!(!line.contains("MiB") && !line.contains("GiB"), "{line}");

    inventory.instances[0].services[0].status = "paused".into();
    app.update_snapshot(inventory);
    let line = toolbar_line(&mut app, 130);
    assert!(line.contains(" — paused"), "{line}");
}

#[test]
fn capital_r_requests_manual_refresh_from_toolbar_and_data_view_at_both_sizes() {
    init_ui();
    for width in [130, 40] {
        let mut app = root(AppService::for_tests());
        let mut layout = tuicore::LayoutCtx::new();
        app.layout(Rect::new(0, 0, width, 30), &mut layout);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "running-only")
            })
            .unwrap()
            .clone();
        app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
        for key in [Key::Char('R')] {
            let mut ctx = EventCtx::new(AnimationSettings::default());
            app.dispatch_event(
                &tuicore::EventRoute::new(target.path.clone()),
                &TuiEvent::Key(KeyEvent::from(key)),
                &mut ctx,
            );
            assert!(matches!(ctx.messages(), [Msg::Refresh]));
            app.handle_message(Msg::Refresh, &mut ctx);
            assert!(app.manual_refresh.is_some());
            app.manual_refresh = None;
        }
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char('R'))),
            &mut EventCtx::new(AnimationSettings::default()),
        );
        assert!(app.manual_refresh.is_some());
    }
}

#[test]
fn escape_from_toolbar_returns_focus_to_the_data_view() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let area = Rect::new(0, 0, 130, 30);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(area, &mut layout);
    let toolbar_targets = layout
        .focus_targets()
        .iter()
        .filter(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| matches!(key.as_str(), "stop-all" | "purge-all"))
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(toolbar_targets.len(), 2);

    for target in toolbar_targets {
        for key in [
            KeyEvent::from(Key::Esc),
            KeyEvent {
                code: Key::Char('['),
                modifiers: KeyModifiers::CONTROL,
            },
        ] {
            let mut ctx = EventCtx::new(AnimationSettings::default());
            app.dispatch_event(
                &tuicore::EventRoute::new(target.path.clone()),
                &TuiEvent::Key(key),
                &mut ctx,
            );
            assert_eq!(ctx.focus_request(), Some(&super::super::initial_focus()));
        }
    }
}

#[test]
fn escape_in_the_data_view_keeps_its_focus() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let area = Rect::new(0, 0, 130, 30);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(area, &mut layout);
    let data_view = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == "environments")
        .unwrap()
        .clone();

    for key in [
        KeyEvent::from(Key::Esc),
        KeyEvent {
            code: Key::Char('['),
            modifiers: KeyModifiers::CONTROL,
        },
    ] {
        let mut ctx = EventCtx::new(AnimationSettings::default());
        let outcome = app.dispatch_event(
            &tuicore::EventRoute::new(data_view.path.clone()),
            &TuiEvent::Key(key),
            &mut ctx,
        );
        assert_eq!(outcome, tuicore::EventOutcome::Handled);
        assert_eq!(ctx.focus_request(), Some(&super::super::initial_focus()));
    }
}

#[test]
fn toolbar_manual_refresh_honors_a_configured_hotkey() {
    init_ui();
    let mut toolbar = crate::app::toolbar::Toolbar::new(
        tuicore::KeySpec::shifted('g'),
        tuicore::KeySpec::shifted('s'),
        tuicore::KeySpec::shifted('p'),
        Default::default(),
        false,
    );
    let area = Rect::new(0, 0, 40, 1);
    toolbar.layout(area, &mut tuicore::LayoutCtx::new());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    toolbar.event(&TuiEvent::Key(KeyEvent::from(Key::Char('G'))), &mut ctx);
    assert!(matches!(ctx.messages(), [Msg::Refresh]));
}

#[test]
fn history_toggle_beside_running_filter_controls_saved_sessions_across_instances() {
    use crate::store::opencode::{Activity, Session, Snapshot};
    init_ui();
    let mut app = root(AppService::for_tests());
    let mut inventory = snapshot();
    let mut other = inventory.instances[0].clone();
    other.name = "other".into();
    other.workspace = "/tmp/workspaces/other".into();
    inventory.instances.push(other);
    app.service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: inventory
            .instances
            .iter()
            .map(|instance| Session {
                id: format!("ses_{}", instance.name),
                title: "Saved conversation".into(),
                directory: instance.workspace.clone(),
                activity: Activity::Idle,
                ..Default::default()
            })
            .collect(),
        clients: Vec::new(),
        error: None,
        ..Default::default()
    });
    app.update_snapshot(inventory.clone());
    for width in [40, 130] {
        let mut layout = tuicore::LayoutCtx::new();
        app.layout(Rect::new(0, 0, width, 30), &mut layout);
        let target = |name: &str| {
            layout
                .focus_targets()
                .iter()
                .find(|target| target.path.keys().iter().any(|key| key.as_str() == name))
                .unwrap()
                .clone()
        };
        let history = target("opencode-history");
        assert_eq!(target("running-only").area.right() + 1, history.area.x);
        let line = toolbar_line(&mut app, width);
        assert!(line.contains("󰋚"), "{line}");
        if width >= super::super::MOBILE_TABS_WIDTH {
            assert!(line.contains("|O|"), "{line}");
        }
        for enabled in [true, false] {
            let mut ctx = EventCtx::new(AnimationSettings::default());
            app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('O'))), &mut ctx);
            assert!(ctx.messages().is_empty());
            assert_eq!(app.opencode_history, enabled);
            for instance in ["review", "other"] {
                super::super::instances::set_highlighted(
                    &app.instances,
                    Some(format!("opencode:{instance}:ses_{instance}")),
                );
                assert_eq!(app.selected().is_some(), enabled);
            }
        }
    }
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(app.service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    app.update_snapshot(inventory);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(Rect::new(0, 0, 130, 30), &mut layout);
    assert!(!layout.focus_targets().iter().any(|target| {
        target
            .path
            .keys()
            .iter()
            .any(|key| key.as_str() == "opencode-history")
    }));
    assert!(!toolbar_line(&mut app, 130).contains("󰋚"));
}
