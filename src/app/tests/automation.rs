use super::*;

#[test]
fn disabled_automation_keeps_navigation_on_workspace_tabs() {
    init_ui();
    let service = AppService::for_tests_with_automation(false);
    let mut app = crate::app::root(service);
    app.update_snapshot(snapshot());
    let mut ctx = EventCtx::default();
    for (key, index, sessions) in [(']', 1, false), (']', 0, true), ('[', 1, false)] {
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Char(key))), &mut ctx);
        let (_, text) = super::events::render(&mut app, 130);
        let header = text.lines().next().unwrap();
        assert!(header.contains("Sessions · Instances"), "{text}");
        for title in ["Events", "Rules", "Streams"] {
            assert!(!text.contains(title), "{text}");
        }
        assert_eq!(app.tabs_mut().selected_index(), index);
        assert_eq!(app.attached_sessions_only, sessions);
        assert!(!app.events_active && !app.rules_active && !app.providers_active);
    }
    app.handle_message(Msg::OpenSettings, &mut ctx);
    let (_, text) = super::events::render(&mut app, 130);
    assert!(text.contains("Instance ping sound"), "{text}");
    assert!(!text.contains("Event acceptance sound"), "{text}");
    app.handle_message(Msg::Close, &mut ctx);
    app.handle_message(Msg::OpenSoundMenu, &mut ctx);
    let (_, text) = super::events::render(&mut app, 130);
    assert!(text.contains("Session completion"), "{text}");
    assert!(text.contains("Instance ping"), "{text}");
    assert!(!text.contains("Rule acceptance"), "{text}");
}

#[test]
fn disabled_automation_without_opencode_has_one_instances_tab() {
    init_ui();
    let service = AppService::for_tests_with_automation(false);
    service
        .set_opencode_enabled(false)
        .unwrap()
        .blocking_recv()
        .unwrap()
        .unwrap();
    let mut app = crate::app::root(service);
    app.update_snapshot(snapshot());
    let mut ctx = EventCtx::default();
    for key in [']', '[', ']'] {
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Char(key))), &mut ctx);
        let (_, text) = super::events::render(&mut app, 130);
        let header = text.lines().next().unwrap();
        assert!(header.contains("Instances"), "{text}");
        for title in ["Sessions", "Events", "Rules", "Streams"] {
            assert!(!header.contains(title), "{text}");
        }
        assert_eq!(app.tabs_mut().selected_index(), 0);
        assert!(!app.attached_sessions_only);
    }
}
