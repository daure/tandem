use std::time::{Duration, Instant};

use tuicore::{EventRoute, LayoutEngine, TreeDispatcher};

use super::*;

fn popup_route(app: &mut super::super::App, area: Rect) -> EventRoute {
    let mut layout = LayoutEngine::new();
    layout.layout(app, area);
    EventRoute::new(layout.overlays().last().unwrap().route_path.clone())
}

fn rendered_app(app: &mut super::super::App, area: Rect) -> String {
    LayoutEngine::new().layout(app, area);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    rendered_lines(&terminal, area).join("\n")
}

#[test]
fn unfocus_keys_clear_an_applied_instance_search() {
    init_ui();
    for key in [
        KeyEvent::from(Key::Esc),
        KeyEvent {
            code: Key::Char('['),
            modifiers: KeyModifiers::CONTROL,
        },
    ] {
        let mut app = root(AppService::for_tests());
        app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
        let area = Rect::new(0, 0, 130, 40);
        let settings = AnimationSettings::default();
        let mut layout = LayoutEngine::new();
        layout.layout(&mut app, area);
        let tree = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == super::super::TREE_FOCUS)
            .unwrap()
            .clone();
        app.dispatch_focus(&tree, true, &mut tuicore::FocusCtx::default());
        let route = EventRoute::new(tree.path);
        for character in "/zzzz".chars() {
            app.dispatch_event(
                &route,
                &TuiEvent::Key(KeyEvent::from(Key::Char(character))),
                &mut EventCtx::new(settings),
            );
        }
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Enter)),
            &mut EventCtx::new(settings),
        );
        assert!(!super::super::instances::is_searching(&app.instances));
        let filtered = rendered_app(&mut app, area);
        assert!(!filtered.contains("review"), "{filtered}");

        app.dispatch_event(&route, &TuiEvent::Key(key), &mut EventCtx::new(settings));

        let cleared = rendered_app(&mut app, area);
        assert!(cleared.contains("review"), "{cleared}");
    }
}

#[test]
fn status_bar_search_keeps_app_shortcuts_as_text() {
    init_ui();
    let mut app = root(AppService::for_tests());
    assert!(app.service.branch_instances());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    let area = Rect::new(0, 0, 130, 40);
    let settings = AnimationSettings {
        enabled: false,
        ..AnimationSettings::default()
    };
    LayoutEngine::new().layout(&mut app, area);
    app.status_bar_mut()
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
        assert!(!app.view.is_active(), "typed {character}");
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
    init_ui();
    let mut app = root(AppService::for_tests());
    let area = Rect::new(0, 0, 130, 40);
    let settings = AnimationSettings {
        enabled: false,
        ..AnimationSettings::default()
    };
    LayoutEngine::new().layout(&mut app, area);
    app.status_bar_mut()
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
fn status_menu_opens_notification_sounds_with_a_right_aligned_shortcut() {
    init_ui();
    for width in [40, 130] {
        let mut app = root(AppService::for_tests());
        let area = Rect::new(0, 0, width, 30);
        let mut layout = LayoutEngine::new();
        layout.layout(&mut app, area);
        let trigger = layout
            .focus_targets()
            .iter()
            .find(|target| target.hotkey_sequences.iter().any(|key| key == ";"))
            .unwrap();
        app.dispatch_event(
            &EventRoute::new(trigger.path.clone()),
            &TuiEvent::Hotkey(HotkeyEvent::Commit(";".into())),
            &mut EventCtx::default(),
        );
        layout.layout(&mut app, area);
        let popup = layout.overlays().last().unwrap().clone();
        let mut terminal = Terminal::new(TestBackend::new(width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut render = RenderCtx::new();
                app.render(frame, area, &mut render);
                render.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        let y = lines
            .iter()
            .position(|line| line.contains("󰕾 Notifications"))
            .unwrap() as u16;
        let hint = terminal
            .backend()
            .buffer()
            .cell((popup.area.right() - 1, y))
            .unwrap();
        assert_eq!(hint.symbol(), "N");
        assert_eq!(hint.fg, tuicore::theme().muted_fg());
        let route = EventRoute::new(popup.route_path);
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent {
                code: Key::Char('j'),
                modifiers: KeyModifiers::CONTROL,
            }),
            &mut EventCtx::default(),
        );
        let mut ctx = EventCtx::default();
        app.dispatch_event(&route, &TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut ctx);
        assert!(matches!(ctx.messages(), [Msg::OpenSoundMenu]));
        app.handle_message(Msg::OpenSoundMenu, &mut ctx);
        let text = rendered_app(&mut app, area);
        let session = text.find("Session completion").unwrap();
        let rule = text.find("Rule acceptance").unwrap();
        let instance = text.find("Instance ping").unwrap();
        assert!(session < rule && rule < instance, "{text}");
    }
}

#[test]
fn settings_duration_input_accepts_digits_and_persists_the_value() {
    init_ui();
    let mut app = root(AppService::for_tests());
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
                .any(|key| key.as_str() == "completion-fade")
        })
        .unwrap();
    let route = EventRoute::new(input.path.clone());
    let mut input_ctx = EventCtx::new(settings);
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut input_ctx,
    );
    app.dispatch_event(&route, &TuiEvent::Paste("4x5".into()), &mut input_ctx);
    assert!(input_ctx.messages().is_empty());
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Home)),
        &mut input_ctx,
    );
    for _ in 0..2 {
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Delete)),
            &mut input_ctx,
        );
    }
    let mut input_ctx = EventCtx::new(settings);
    app.dispatch_event(&route, &TuiEvent::Paste("45".into()), &mut input_ctx);
    let value = input_ctx
        .messages()
        .iter()
        .find_map(|message| match message {
            Msg::CompletionFadeChanged(value) => Some(value.clone()),
            _ => None,
        })
        .expect("editing the input emits its duration");
    assert_eq!(value, "45");
    app.handle_message(
        Msg::CompletionFadeChanged(value),
        &mut EventCtx::new(settings),
    );
    app.service.flush_settings();
    assert_eq!(app.service.completion_fade_seconds(), 45);
    app.handle_message(Msg::Close, &mut EventCtx::new(settings));
    app.handle_message(Msg::OpenSettings, &mut EventCtx::new(settings));
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
    assert!(text.contains("Completion fade"), "{text}");
    assert!(text.contains("45"), "{text}");
    assert!(text.contains("Completion sound"), "{text}");
}

#[test]
fn sound_dropdown_previews_open_and_close_and_saves_only_confirmed_changes() {
    init_ui();
    for field_key in [
        "completion-sound",
        "event-acceptance-sound",
        "instance-ping-sound",
    ] {
        let mut service = AppService::for_tests();
        service.set_sound_choices_for_tests(vec![
            crate::store::completion::SoundChoice {
                id: String::new(),
                label: "System default".into(),
            },
            crate::store::completion::SoundChoice {
                id: "/sounds/bell.oga".into(),
                label: "Bell".into(),
            },
        ]);
        let mut app = root(service);
        let settings = AnimationSettings {
            enabled: false,
            ..Default::default()
        };
        app.handle_message(Msg::OpenSettings, &mut EventCtx::new(settings));
        let area = Rect::new(0, 0, 130, 40);
        let mut layout = LayoutEngine::new();
        layout.layout(&mut app, area);
        let field = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == field_key)
            })
            .unwrap();
        app.dispatch_focus(field, true, &mut tuicore::FocusCtx::default());
        app.dispatch_event(
            &EventRoute::new(field.path.clone()),
            &TuiEvent::Key(KeyEvent::from(Key::Enter)),
            &mut EventCtx::new(settings),
        );
        assert_eq!(app.service.completion_sound_count_for_tests(), 1);
        let route = popup_route(&mut app, area);
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent {
                code: Key::Char('j'),
                modifiers: KeyModifiers::CONTROL,
            }),
            &mut EventCtx::new(settings),
        );
        assert_eq!(app.service.completion_sound_count_for_tests(), 1);
        assert_eq!(app.service.completion_sound_choice(), "");
        assert_eq!(app.service.event_acceptance_sound_choice(), "");
        assert_eq!(app.service.instance_ping_sound_choice(), "");
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Enter)),
            &mut EventCtx::new(settings),
        );
        assert!(app.settings_save.is_some());
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.settings_save.is_some() {
            assert!(Instant::now() < deadline, "sound setting was not saved");
            app.tick(Duration::from_millis(10), settings);
            std::thread::sleep(Duration::from_millis(5));
        }
        let completion = if field_key == "completion-sound" {
            "/sounds/bell.oga"
        } else {
            ""
        };
        let acceptance = if field_key == "event-acceptance-sound" {
            "/sounds/bell.oga"
        } else {
            ""
        };
        let ping = if field_key == "instance-ping-sound" {
            "/sounds/bell.oga"
        } else {
            ""
        };
        assert_eq!(app.service.completion_sound_choice(), completion);
        assert_eq!(app.service.event_acceptance_sound_choice(), acceptance);
        assert_eq!(app.service.instance_ping_sound_choice(), ping);
        assert_eq!(app.service.completion_sound_count_for_tests(), 2);
        assert!(!app.completion_sound);
        let text = rendered_app(&mut app, area);
        let bottom = text
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix('╰')?
                    .split_once('╯')
                    .map(|(border, _)| border)
            })
            .expect("settings has a bottom border");
        assert!(bottom.chars().all(|character| character == '─'), "{text}");

        let field_route = EventRoute::new(field.path.clone());
        for close in [Key::Enter, Key::Esc] {
            app.dispatch_event(
                &field_route,
                &TuiEvent::Key(KeyEvent::from(Key::Enter)),
                &mut EventCtx::new(settings),
            );
            let route = popup_route(&mut app, area);
            assert_eq!(
                app.service.completion_sound_count_for_tests(),
                if close == Key::Enter { 3 } else { 5 }
            );
            if close == Key::Esc {
                app.dispatch_event(
                    &route,
                    &TuiEvent::Key(KeyEvent {
                        code: Key::Char('k'),
                        modifiers: KeyModifiers::CONTROL,
                    }),
                    &mut EventCtx::new(settings),
                );
            }
            app.dispatch_event(
                &route,
                &TuiEvent::Key(KeyEvent::from(close)),
                &mut EventCtx::new(settings),
            );
            assert!(app.settings_save.is_none());
        }
        assert_eq!(app.service.completion_sound_count_for_tests(), 6);
        assert_eq!(app.service.completion_sound_choice(), completion);
        assert_eq!(app.service.event_acceptance_sound_choice(), acceptance);
        assert_eq!(app.service.instance_ping_sound_choice(), ping);
    }
}
