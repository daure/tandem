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
    assert!(!app.completion_sound);
}

#[test]
fn events_sound_is_the_first_toggle_and_overview_mutes_it_in_both_integration_modes() {
    for opencode in [true, false] {
        let service = AppService::for_tests();
        service
            .set_opencode_enabled(opencode)
            .unwrap()
            .blocking_recv()
            .unwrap()
            .unwrap();
        let mut app = super::super::root(service);
        app.tabs_mut().select_index(1);
        app.sync_overview_tab(&mut EventCtx::default());
        for width in [40, 130] {
            let (layout, text) = super::events::render(&mut app, width);
            assert!(text.contains("󰕾 |N|"), "{text}");
            let target = |hotkey: &str| {
                layout
                    .focus_targets()
                    .iter()
                    .find(|target| target.hotkey_sequences.iter().any(|key| key == hotkey))
                    .unwrap()
            };
            let sound = target("shift+n");
            assert!(sound.area.x < target("shift+t").area.x, "{text}");
            assert!(sound.area.x < target("gg").area.x, "{text}");
            let feed = layout
                .focus_targets()
                .iter()
                .find(|target| target.id.as_str() == crate::app::events::FOCUS)
                .unwrap();
            app.dispatch_focus(feed, true, &mut tuicore::FocusCtx::default());
            let mut ctx = EventCtx::default();
            app.dispatch_event(
                &EventRoute::new(feed.path.clone()),
                &TuiEvent::Key(KeyEvent::from(Key::Char('N'))),
                &mut ctx,
            );
            for message in ctx.drain_messages() {
                app.handle_message(message, &mut EventCtx::default());
            }
            assert!(app.event_acceptance_sound);
            assert!(app.toolbar_state.borrow().event_acceptance_sound);
            assert!(!app.completion_sound);
            assert_eq!(ctx.focus_request(), Some(&tuicore::FocusRequest::Keep));
            app.update_snapshot(snapshot());
            assert!(app.toolbar_state.borrow().event_acceptance_sound);
            app.event(
                &TuiEvent::Hotkey(HotkeyEvent::Commit("shift+h".into())),
                &mut EventCtx::default(),
            );
            assert!(!app.event_acceptance_sound);
            assert!(!app.toolbar_state.borrow().event_acceptance_sound);
            assert_eq!(app.tabs_mut().selected_index(), 0);
            app.tabs_mut().select_index(1);
            app.sync_overview_tab(&mut EventCtx::default());
        }
    }
}
