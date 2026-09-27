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
    tuicore::init();
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
    tuicore::init();
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
fn settings_duration_input_accepts_digits_and_persists_the_value() {
    tuicore::init();
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
fn sound_dropdown_previews_and_saves_only_when_a_choice_is_confirmed() {
    tuicore::init();
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
                .any(|key| key.as_str() == "completion-sound")
        })
        .unwrap();
    app.dispatch_focus(field, true, &mut tuicore::FocusCtx::default());
    app.dispatch_event(
        &EventRoute::new(field.path.clone()),
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::new(settings),
    );
    let route = popup_route(&mut app, area);
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent {
            code: Key::Char('j'),
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut EventCtx::new(settings),
    );
    assert_eq!(app.service.completion_sound_count_for_tests(), 0);
    assert_eq!(app.service.completion_sound_choice(), "");
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut EventCtx::new(settings),
    );
    app.settings_save
        .take()
        .expect("confirming a new sound saves it")
        .blocking_recv()
        .unwrap()
        .unwrap();
    assert_eq!(app.service.completion_sound_choice(), "/sounds/bell.oga");
    assert_eq!(app.service.completion_sound_count_for_tests(), 1);
    assert!(!app.completion_sound);
}
