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
fn running_filter_toggle_precedes_stop_all_and_filters_with_capital_u() {
    tuicore::init();
    let mut inventory = snapshot();
    let mut other = inventory.instances[0].clone();
    other.name = "other".into();
    inventory.instances.push(other);
    let mut stopped = inventory.instances[0].clone();
    stopped.name = "stopped".into();
    stopped.services[0].status = "down (exit 0)".into();
    inventory.instances.push(stopped);
    let mut app = root(AppService::for_tests());
    app.update_snapshot(inventory.clone());

    for width in [40, 130] {
        let area = Rect::new(0, 0, width, 30);
        let mut layout = tuicore::LayoutCtx::new();
        app.layout(area, &mut layout);
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
        let stop = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "stop-all")
            })
            .unwrap();
        assert_eq!(running.area.right(), stop.area.x);
        let line = toolbar_line(&mut app, width);
        assert!(line.contains("󰑮") && line.contains("|U|"), "{line}");
        assert!(line.find("󰑮").unwrap() < line.find('').unwrap(), "{line}");
    }

    let all = rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30)).join("\n");
    assert!(
        all.contains("review") && all.contains("other") && all.contains("stopped"),
        "{all}"
    );
    assert!(all.contains(" 3"), "{all}");

    let mut toggle = EventCtx::new(AnimationSettings::default());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('U'))), &mut toggle);
    assert!(toggle.messages().is_empty());
    assert!(app.running_only);
    let running =
        rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30)).join("\n");
    assert!(running.contains("review"), "{running}");
    assert!(running.contains("other"), "{running}");
    assert!(!running.contains("stopped"), "{running}");
    assert!(running.contains(" 2/3"), "{running}");

    inventory
        .instances
        .iter_mut()
        .find(|instance| instance.name == "stopped")
        .unwrap()
        .services[0]
        .status = "up".into();
    app.update_snapshot(inventory.clone());
    let running =
        rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30)).join("\n");
    assert!(running.contains(" 3"), "{running}");
    assert!(!running.contains("2/3"), "{running}");

    let mut toggle = EventCtx::new(AnimationSettings::default());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('U'))), &mut toggle);
    assert!(toggle.messages().is_empty());
    assert!(!app.running_only);
    inventory
        .instances
        .iter_mut()
        .find(|instance| instance.name == "stopped")
        .unwrap()
        .services[0]
        .status = "down (exit 0)".into();
    app.update_snapshot(inventory);
    let all = rendered_lines(&toolbar_terminal(&mut app, 130), Rect::new(0, 0, 130, 30)).join("\n");
    assert!(all.contains(" 3"), "{all}");
    assert!(!all.contains("2/3"), "{all}");
}

#[test]
fn toolbar_totals_cover_all_instances_and_update_independently_of_tree_search() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let mut inventory = snapshot();
    inventory.available_memory_bytes = Some(8 * 1073741824);
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

    for width in [80, 130] {
        let line = toolbar_line(&mut app, width);
        assert!(line.contains(" 8 GiB ·  1 GiB · 500%"), "{line}");
        for label in ["used", "available", "CPU"] {
            assert!(!line.contains(label), "{line}");
        }
        assert!(line.find("500%").unwrap() < line.find("󰑓").unwrap());
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
    assert!(toolbar_line(&mut app, 130).contains(" 8 GiB ·  1 GiB · 500%"));

    inventory.instances.pop();
    app.update_snapshot(inventory);
    assert!(
        app.view
            .tick(std::time::Duration::ZERO, AnimationSettings::default())
            .layout
    );
    assert!(toolbar_line(&mut app, 130).contains(" 8 GiB ·  0.5 GiB · 250%"));
}

#[test]
fn toolbar_totals_show_unavailable_and_paused_states() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.update_snapshot(Default::default());
    let line = toolbar_line(&mut app, 130);
    assert!(line.contains(" —"), "{line}");
    assert!(!line.contains(" · "), "{line}");

    let mut guidance_only = snapshot();
    guidance_only.instances[0].services.clear();
    guidance_only.instances[0].workspace_only = true;
    guidance_only.instances[0].runtime.workspace_ready = true;
    app.update_snapshot(guidance_only);
    let line = toolbar_line(&mut app, 130);
    assert!(!line.contains(" · "), "{line}");

    let spinner = tuicore::Spinner::new().glyph().to_owned();
    let mut initial = snapshot();
    initial.available_memory_bytes = Some(20 * 1073741824);
    app.update_snapshot(initial);
    let line = toolbar_line(&mut app, 130);
    assert!(line.contains(&spinner), "{line}");
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
        line.contains(&format!(" — ·  20 MiB · 500% {spinner}")),
        "{line}"
    );
    let terminal = toolbar_terminal(&mut app, 130);
    let x = line.chars().position(|character| character == '').unwrap() as u16;
    assert_eq!(
        terminal.backend().buffer().cell((x, 1)).unwrap().fg,
        tuicore::theme().muted_fg()
    );
    let narrow = toolbar_line(&mut app, 40);
    assert!(narrow.contains("󰠲 T") && narrow.contains("󰑓 R"), "{narrow}");
    assert!(
        !narrow.contains("MiB"),
        "unavailable totals should remain hidden when they do not fit: {narrow}"
    );

    inventory.instances.pop();
    inventory.instances[0].services[0]
        .usage
        .as_mut()
        .unwrap()
        .cpu_basis_points = None;
    app.update_snapshot(inventory.clone());
    let line = toolbar_line(&mut app, 130);
    assert!(line.contains(&spinner), "{line}");
    assert!(!line.contains("MiB") && !line.contains(''), "{line}");

    inventory.instances[0].services[0].status = "paused".into();
    app.update_snapshot(inventory);
    let line = toolbar_line(&mut app, 130);
    assert!(line.contains(" — · paused"), "{line}");
    assert!(!line.contains(" · "), "{line}");
}

#[test]
fn refresh_button_is_rightmost_and_shows_its_hotkey_at_both_sizes() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    for width in [130, 99, 40, 100, 150] {
        let area = Rect::new(0, 0, width, 30);
        let mut layout = tuicore::LayoutCtx::new();
        app.layout(area, &mut layout);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "refresh")
            })
            .unwrap();
        assert_eq!(target.area.right(), area.right());
        let button_order = layout
            .focus_targets()
            .iter()
            .flat_map(|target| target.path.keys())
            .filter(|key| matches!(key.as_str(), "stop-all" | "purge-all" | "refresh"))
            .map(|key| key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(button_order, ["stop-all", "purge-all", "refresh"]);
        assert_eq!(target.area.y, 1);
        let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut render = RenderCtx::new();
                app.render(frame, area, &mut render);
                render.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        assert!(lines[1].trim_end().ends_with(if width < 100 {
            "󰑓 R"
        } else {
            "󰑓 Refresh"
        }));
        assert!(lines[1].find('').unwrap() < lines[1].find('').unwrap());
        assert!(lines[1].find('').unwrap() < lines[1].find('󰑓').unwrap());
        let label: String = lines[usize::from(target.area.y)]
            .chars()
            .skip(usize::from(target.area.x))
            .take(usize::from(target.area.width))
            .collect();
        assert_eq!(
            label.trim(),
            if width < 100 {
                "󰑓 R"
            } else {
                "󰑓 Refresh"
            }
        );
    }
}

#[test]
fn toolbar_refresh_activation_and_capital_r_request_manual_refresh_at_both_sizes() {
    tuicore::init();
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
                    .any(|key| key.as_str() == "refresh")
            })
            .unwrap()
            .clone();
        app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
        for key in [Key::Enter, Key::Char('R')] {
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
    tuicore::init();
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
                .any(|key| matches!(key.as_str(), "new-template" | "refresh"))
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
    tuicore::init();
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
fn compact_refresh_button_honors_a_configured_hotkey() {
    tuicore::init();
    let mut toolbar = crate::app::toolbar::Toolbar::new(
        tuicore::KeySpec::shifted('t'),
        tuicore::KeySpec::shifted('g'),
        tuicore::KeySpec::shifted('s'),
        tuicore::KeySpec::shifted('p'),
        Default::default(),
    );
    let area = Rect::new(0, 0, 40, 1);
    toolbar.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(40, 1)).unwrap();
    terminal
        .draw(|frame| toolbar.render(frame, area, &mut RenderCtx::new()))
        .unwrap();
    assert!(
        rendered_lines(&terminal, area)[0]
            .trim_end()
            .contains("󰑓 G")
    );
    let mut ctx = EventCtx::new(AnimationSettings::default());
    toolbar.event(&TuiEvent::Key(KeyEvent::from(Key::Char('G'))), &mut ctx);
    assert!(matches!(ctx.messages(), [Msg::Refresh]));
}

#[test]
fn history_toggle_beside_running_filter_controls_saved_sessions_across_instances() {
    use crate::store::opencode::{Activity, Session, Snapshot};
    tuicore::init();
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
        assert_eq!(history.area.right() + 1, target("running-only").area.x);
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
