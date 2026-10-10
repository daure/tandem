use super::*;
use crate::store::events::Snapshot;

fn observed(count: u64) -> Snapshot {
    Snapshot {
        accepted_attempts: Some(count),
        ..Default::default()
    }
}

fn enable(app: &mut App) {
    app.handle_message(Msg::SetEventAcceptanceSound(true), &mut EventCtx::default());
}

#[test]
fn acceptance_sound_observes_new_attempts_without_replaying_history_or_stale_data() {
    let mut app = super::super::root(AppService::for_tests());
    enable(&mut app);
    app.update_event_snapshot(observed(5));
    assert_eq!(app.service.completion_sound_count_for_tests(), 0);
    app.update_event_snapshot(observed(6));
    app.update_event_snapshot(observed(6));
    assert_eq!(app.service.completion_sound_count_for_tests(), 1);

    let mut failed = observed(7);
    failed.error = Some("event observation failed".into());
    app.update_event_snapshot(failed);
    assert_eq!(app.service.completion_sound_count_for_tests(), 1);
    app.update_event_snapshot(observed(7));
    assert_eq!(app.service.completion_sound_count_for_tests(), 2);

    app.handle_message(
        Msg::SetEventAcceptanceSound(false),
        &mut EventCtx::default(),
    );
    app.update_event_snapshot(observed(8));
    enable(&mut app);
    app.update_event_snapshot(observed(8));
    assert_eq!(app.service.completion_sound_count_for_tests(), 2);
    app.update_event_snapshot(observed(9));
    assert_eq!(app.service.completion_sound_count_for_tests(), 3);
    assert!(!app.events_active);
    assert!(app.event_acceptance_sound);
    assert!(!app.completion_sound);
}

fn menu_key(app: &mut App, key: KeyEvent) {
    let area = Rect::new(0, 0, 130, 30);
    let mut layout = LayoutEngine::new();
    layout.layout(app, area);
    let route = EventRoute::new(layout.overlays().last().unwrap().route_path.clone());
    app.dispatch_event(&route, &TuiEvent::Key(key), &mut EventCtx::default());
}

fn open_menu(app: &mut App) {
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('N'))),
        &mut EventCtx::default(),
    );
}

fn commit_menu(app: &mut App) {
    menu_key(
        app,
        KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::CONTROL,
        },
    );
}

fn next_option(app: &mut App) {
    menu_key(
        app,
        KeyEvent {
            code: Key::Char('j'),
            modifiers: KeyModifiers::CONTROL,
        },
    );
}

#[test]
fn notification_menu_commits_independent_gates_and_cancels_drafts() {
    init_ui();
    let mut app = root(AppService::for_tests());
    open_menu(&mut app);
    menu_key(&mut app, KeyEvent::from(Key::Enter));
    assert!(!app.completion_sound);
    commit_menu(&mut app);
    assert!(app.completion_sound);
    assert!(!app.event_acceptance_sound && !app.service.instance_ping_enabled());

    open_menu(&mut app);
    next_option(&mut app);
    menu_key(&mut app, KeyEvent::from(Key::Enter));
    menu_key(&mut app, KeyEvent::from(Key::Esc));
    assert!(app.completion_sound);
    assert!(!app.event_acceptance_sound);

    open_menu(&mut app);
    next_option(&mut app);
    menu_key(&mut app, KeyEvent::from(Key::Enter));
    next_option(&mut app);
    menu_key(&mut app, KeyEvent::from(Key::Enter));
    commit_menu(&mut app);
    app.service.flush_settings();
    assert!(
        app.completion_sound && app.event_acceptance_sound && app.service.instance_ping_enabled()
    );
}

#[test]
fn tui_startup_disables_all_notifications_and_preserves_sound_choices() {
    let mut service = AppService::for_tests();
    let choice = "/sounds/bell.oga".to_string();
    service.set_sound_choices_for_tests(vec![crate::store::completion::SoundChoice {
        id: choice.clone(),
        label: "Bell".into(),
    }]);
    for saved in [
        service.set_completion_sound_choice(choice.clone()),
        service.set_event_acceptance_sound_choice(choice.clone()),
        service.set_instance_ping_sound_choice(choice.clone()),
    ] {
        saved.unwrap().blocking_recv().unwrap().unwrap();
    }
    service
        .set_instance_ping_enabled(true)
        .unwrap()
        .blocking_recv()
        .unwrap()
        .unwrap();
    let config = service.config_for_tests();
    let app = super::super::start(service).unwrap();
    assert!(!app.completion_sound && !app.event_acceptance_sound);
    assert!(!app.service.instance_ping_enabled());
    assert_eq!(app.service.completion_sound_choice(), choice);
    assert_eq!(app.service.event_acceptance_sound_choice(), choice);
    assert_eq!(app.service.instance_ping_sound_choice(), choice);
    assert_eq!(app.service.completion_sound_count_for_tests(), 3);
    let connection = rusqlite::Connection::open(config.home.join("settings.sqlite3")).unwrap();
    let enabled: String = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'instances.ping_enabled'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(enabled, "false");
}

#[test]
fn tui_startup_requires_a_saved_notification_reset() {
    let service = AppService::for_tests();
    let connection =
        rusqlite::Connection::open(service.config_for_tests().home.join("settings.sqlite3"))
            .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_notification_reset BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL, 'read only settings'); END;").unwrap();
    let error = super::super::start(service).err().unwrap();
    assert!(error.contains("read only settings"), "{error}");
}

#[test]
fn global_overview_preserves_notification_enablement_on_every_page() {
    for opencode in [true, false] {
        let service = AppService::for_tests();
        service
            .set_opencode_enabled(opencode)
            .unwrap()
            .blocking_recv()
            .unwrap()
            .unwrap();
        let mut app = root(service);
        app.handle_message(Msg::SetCompletionSound(true), &mut EventCtx::default());
        app.handle_message(Msg::SetEventAcceptanceSound(true), &mut EventCtx::default());
        app.handle_message(Msg::SetInstancePingSound(true), &mut EventCtx::default());
        app.service.flush_settings();
        for page in 0..if opencode { 5 } else { 4 } {
            app.tabs_mut().select_index(page);
            app.sync_overview_tab(&mut EventCtx::default());
            let mut ctx = EventCtx::default();
            app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('H'))), &mut ctx);
            for message in ctx.drain_messages() {
                app.handle_message(message, &mut EventCtx::default());
            }
            assert!(
                app.completion_sound
                    && app.event_acceptance_sound
                    && app.service.instance_ping_enabled()
            );
            assert_eq!(app.tabs_mut().selected_index(), 0);
        }
    }
}

#[test]
fn notification_menu_is_centered_and_available_over_every_page_and_settings() {
    init_ui();
    for opencode in [true, false] {
        let service = AppService::for_tests();
        service
            .set_opencode_enabled(opencode)
            .unwrap()
            .blocking_recv()
            .unwrap()
            .unwrap();
        let mut app = root(service);
        let page_count = if opencode { 5 } else { 4 };
        for page in 0..=page_count {
            if page == page_count {
                app.handle_message(Msg::OpenSettings, &mut EventCtx::default());
            } else {
                app.tabs_mut().select_index(page);
                app.sync_overview_tab(&mut EventCtx::default());
            }
            for width in [40, 130] {
                let mut layout = LayoutEngine::new();
                let area = Rect::new(0, 0, width, 30);
                layout.layout(&mut app, area);
                let target = layout
                    .focus_targets()
                    .iter()
                    .find(|target| target.enabled)
                    .unwrap()
                    .clone();
                let mut focus = tuicore::FocusManager::new();
                focus.apply_request(
                    &tuicore::FocusRequest::TargetAt {
                        path: target.path.clone(),
                        id: target.id.clone(),
                    },
                    layout.focus_targets(),
                );
                app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
                let mut open = EventCtx::default();
                app.dispatch_event(
                    &EventRoute::new(target.path.clone()),
                    &TuiEvent::Key(KeyEvent::from(Key::Char('N'))),
                    &mut open,
                );
                layout.layout(&mut app, area);
                let transition = focus
                    .apply_request(open.focus_request().unwrap(), layout.focus_targets())
                    .unwrap();
                app.dispatch_focus(
                    transition.previous.as_ref().unwrap(),
                    false,
                    &mut tuicore::FocusCtx::default(),
                );
                app.dispatch_focus(
                    transition.current.as_ref().unwrap(),
                    true,
                    &mut tuicore::FocusCtx::default(),
                );
                assert!(
                    focus
                        .current()
                        .unwrap()
                        .path
                        .keys()
                        .iter()
                        .any(|key| key.as_str() == "notification-sounds")
                );
                let popup = layout.overlays().last().unwrap().area;
                assert!((i32::from(popup.x * 2 + popup.width) - i32::from(width)).abs() <= 1);
                assert!((i32::from(popup.y * 2 + popup.height) - 30).abs() <= 1);
                let (_, text) = super::events::render(&mut app, width);
                for label in ["Session completion", "Rule acceptance", "Instance ping"] {
                    assert!(text.contains(label), "{text}");
                }
                let route = EventRoute::new(layout.overlays().last().unwrap().route_path.clone());
                let mut ctx = EventCtx::default();
                app.dispatch_event(&route, &TuiEvent::Key(KeyEvent::from(Key::Esc)), &mut ctx);
                assert_eq!(ctx.focus_request(), Some(&tuicore::FocusRequest::Last));
                layout.layout(&mut app, area);
                let transition = focus
                    .apply_request(ctx.focus_request().unwrap(), layout.focus_targets())
                    .unwrap();
                app.dispatch_focus(
                    transition.previous.as_ref().unwrap(),
                    false,
                    &mut tuicore::FocusCtx::default(),
                );
                app.dispatch_focus(
                    transition.current.as_ref().unwrap(),
                    true,
                    &mut tuicore::FocusCtx::default(),
                );
                assert_eq!(focus.current().unwrap().path, target.path);
                assert_eq!(focus.current().unwrap().id, target.id);
                assert_eq!(app.tabs_mut().selected_index(), page.min(page_count - 1));
                assert_eq!(app.view.is_active(), page == page_count);
            }
        }
    }
}
