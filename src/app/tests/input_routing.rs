use tuicore::{EventRoute, LayoutEngine, TreeDispatcher};

use super::*;

fn popup_route(app: &mut super::super::App, area: Rect) -> EventRoute {
    let mut layout = LayoutEngine::new();
    layout.layout(app, area);
    EventRoute::new(layout.overlays().last().unwrap().route_path.clone())
}

#[test]
fn status_bar_search_keeps_app_shortcuts_as_text() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    assert!(app.service.branch_instances());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);
    let settings = AnimationSettings {
        enabled: false,
        ..AnimationSettings::default()
    };
    LayoutEngine::new().layout(&mut app, area);
    app.view
        .second_mut()
        .toggle_menu(&mut EventCtx::new(settings));
    let route = popup_route(&mut app, area);
    let mut dispatcher = TreeDispatcher::new();

    let query = "vntsdr.";
    for character in query.chars() {
        dispatcher.dispatch_event(
            &mut app,
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
            settings,
        );
        assert!(!app.view.first().is_active(), "typed {character}");
        assert!(!app.menu_layer().is_active(), "typed {character}");
        assert!(app.intent.is_none());
    }
    assert!(app.service.operations().is_empty());
    assert!(app.service.opened_system_targets().is_empty());
    popup_route(&mut app, area);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let text = rendered_lines(&terminal, area).join("\n");
    assert!(text.contains(query), "{text}");
}

#[test]
fn status_bar_menu_opens_branch_instance_settings() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let area = Rect::new(0, 0, 130, 40);
    let settings = AnimationSettings {
        enabled: false,
        ..AnimationSettings::default()
    };
    LayoutEngine::new().layout(&mut app, area);
    app.view
        .second_mut()
        .toggle_menu(&mut EventCtx::new(settings));
    let route = popup_route(&mut app, area);
    let mut ctx = EventCtx::new(settings);

    app.dispatch_event(&route, &TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut ctx);

    assert!(matches!(ctx.messages(), [Msg::OpenSettings]));
    app.handle_message(Msg::OpenSettings, &mut EventCtx::new(settings));
    let mut layout = LayoutEngine::new();
    layout.layout(&mut app, area);
    let toggle = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "branch-instances")
        })
        .unwrap();
    let mut toggle_ctx = EventCtx::new(settings);
    app.dispatch_event(
        &EventRoute::new(toggle.path.clone()),
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut toggle_ctx,
    );
    assert!(matches!(
        toggle_ctx.messages(),
        [Msg::SetBranchInstances(false)]
    ));
    app.handle_message(Msg::SetBranchInstances(false), &mut EventCtx::new(settings));
    app.service.flush_settings();
    assert!(!app.service.branch_instances());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    assert!(
        rendered_lines(&terminal, area)
            .join("\n")
            .contains("Branch instances")
    );
}

#[test]
fn saved_open_command_runs_from_the_instance_shortcut_and_menu() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let workspace = tempfile::tempdir().unwrap();
    let command = "printf '%s' \"$TANDEM_WORKSPACE\" > opened";
    let settings = AnimationSettings {
        enabled: false,
        ..AnimationSettings::default()
    };
    app.handle_message(Msg::OpenSettings, &mut EventCtx::new(settings));
    let area = Rect::new(0, 0, 130, 40);
    let mut layout = LayoutEngine::new();
    layout.layout(&mut app, area);
    let input = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "open-command")
        })
        .unwrap();
    let route = EventRoute::new(input.path.clone());
    let mut input_ctx = EventCtx::new(settings);
    app.dispatch_event(&route, &TuiEvent::Paste(command.into()), &mut input_ctx);
    let value = input_ctx
        .messages()
        .iter()
        .find_map(|message| match message {
            Msg::OpenCommandChanged(value) => Some(value.clone()),
            _ => None,
        })
        .expect("editing the input emits its command");
    assert_eq!(value, command);
    app.handle_message(Msg::OpenCommandChanged(value), &mut EventCtx::new(settings));
    app.service.flush_settings();
    assert_eq!(app.service.open_command(), command);
    assert!(!workspace.path().join("opened").exists());
    app.handle_message(Msg::Close, &mut EventCtx::new(settings));
    app.handle_message(Msg::OpenSettings, &mut EventCtx::new(settings));
    assert_eq!(app.open_command, command);
    layout.layout(&mut app, area);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let text = rendered_lines(&terminal, area).join("\n");
    assert!(text.contains("Open command"), "{text}");
    assert!(text.contains(command), "{text}");

    app.handle_message(Msg::Close, &mut EventCtx::new(settings));
    let mut snapshot = snapshot();
    snapshot.instances[0].workspace = workspace.path().to_str().unwrap().into();
    app.set_rows_for_tests(rows::from_snapshot(&snapshot));
    super::super::instances::set_highlighted(&app.instances, Some("instance:review".into()));
    for menu in [false, true] {
        if menu {
            for character in ".Run open command".chars() {
                app.event(
                    &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
                    &mut EventCtx::new(settings),
                );
            }
            app.event(
                &TuiEvent::Key(KeyEvent::from(Key::Enter)),
                &mut EventCtx::new(settings),
            );
            assert!(!app.menu_layer().is_active());
        } else {
            app.event(
                &TuiEvent::Key(KeyEvent {
                    code: Key::Char(';'),
                    modifiers: KeyModifiers::CONTROL,
                }),
                &mut EventCtx::new(settings),
            );
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if std::fs::read_to_string(workspace.path().join("opened"))
                .ok()
                .as_deref()
                == workspace.path().to_str()
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "custom workspace command did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::remove_file(workspace.path().join("opened")).unwrap();
        assert!(!app.view.first().is_active());
    }
    assert!(app.service.opened_system_targets().is_empty());
}
