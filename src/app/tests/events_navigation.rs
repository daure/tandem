use super::*;
use crate::store::events::{Record, Snapshot};
use std::time::Duration;

fn update_events(app: &mut App, count: i64) {
    let records = (1..=count)
        .rev()
        .map(|sequence| {
            let id = format!("event-{sequence:03}");
            let mut event = crate::environments::events::tests::event(&id);
            if let crate::store::events::Payload::Message(message) = &mut event.payload {
                message.text = id;
            }
            Record {
                sequence,
                provider: if sequence % 2 == 0 { "alpha" } else { "beta" }.into(),
                received_at: "now".into(),
                event,
                attempts: vec![],
            }
        })
        .collect();
    app.pages_mut().update_events(Snapshot {
        records,
        total: count as u64,
        ..Default::default()
    });
    app.pages_mut().tick(
        Duration::ZERO,
        AnimationSettings {
            enabled: false,
            ..Default::default()
        },
    );
}

fn render(app: &mut App) -> (tuicore::LayoutCtx, String) {
    let area = Rect::new(0, 0, 130, 30);
    let mut layout = tuicore::LayoutCtx::new();
    layout.with_overlay_bounds(area, |ctx| app.layout(area, ctx));
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            app.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    (layout, rendered_lines(&terminal, area).join("\n"))
}

fn focus_feed(app: &mut App) -> EventRoute {
    let (layout, _) = render(app);
    let target = focus_target(&layout, crate::app::events::FOCUS);
    app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
    EventRoute::new(target.path.clone())
}

fn focus_target(layout: &tuicore::LayoutCtx, id: &str) -> tuicore::FocusTarget {
    layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == id)
        .unwrap()
        .clone()
}

fn control(layout: &tuicore::LayoutCtx, hotkey: &str) -> tuicore::FocusTarget {
    layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .hotkey_sequences
                .iter()
                .any(|sequence| sequence == hotkey)
        })
        .unwrap()
        .clone()
}

fn shifted(character: char) -> TuiEvent {
    TuiEvent::Key(KeyEvent {
        code: Key::Char(character.to_ascii_uppercase()),
        modifiers: KeyModifiers::SHIFT,
    })
}

fn hotkey(sequence: &str) -> TuiEvent {
    TuiEvent::Hotkey(HotkeyEvent::Commit(sequence.into()))
}

fn control_bracket() -> TuiEvent {
    TuiEvent::Key(KeyEvent {
        code: Key::Char('['),
        modifiers: KeyModifiers::CONTROL,
    })
}

fn selected(app: &mut App, route: &EventRoute) -> i64 {
    let mut ctx = EventCtx::default();
    app.dispatch_event(route, &TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut ctx);
    match ctx.messages() {
        [Msg::OpenEvent(row)] => row.sequence,
        other => panic!("Unexpected messages: {other:?}"),
    }
}

fn send(app: &mut App, route: &EventRoute, event: TuiEvent) -> EventCtx<Msg> {
    let mut ctx = EventCtx::new(AnimationSettings {
        enabled: false,
        ..Default::default()
    });
    app.dispatch_event(route, &event, &mut ctx);
    ctx
}

fn assert_feed_focus(ctx: &EventCtx<Msg>) {
    assert!(
        matches!(ctx.focus_request(), Some(tuicore::FocusRequest::Target(id)) if id.as_str() == crate::app::events::FOCUS),
        "{:?}",
        ctx.focus_request()
    );
}

fn events_app(count: i64) -> App {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    update_events(&mut app, count);
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
        &mut EventCtx::default(),
    );
    app
}

fn select_tab(app: &mut App, index: usize) {
    while app.tabs_mut().selected_index() != index {
        app.event(
            &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
            &mut EventCtx::default(),
        );
    }
}

#[test]
fn events_follow_the_newest_row_until_navigation_leaves_the_bottom() {
    let mut app = events_app(20);
    let route = focus_feed(&mut app);
    assert_eq!(selected(&mut app, &route), 20);
    let text = render(&mut app).1;
    assert!(text.contains("──●  |G|"), "{text}");
    assert!(text.contains("event-020"), "{text}");
    assert!(!text.contains("event-001"), "{text}");
    assert!(
        text.find("event-019").unwrap() < text.find("event-020").unwrap(),
        "{text}"
    );

    update_events(&mut app, 21);
    assert_eq!(selected(&mut app, &route), 21);
    assert!(render(&mut app).1.contains("event-021"));
    send(&mut app, &route, TuiEvent::Key(KeyEvent::from(Key::Up)));
    assert_eq!(selected(&mut app, &route), 20);
    assert!(render(&mut app).1.contains("○──  |G|"));
    update_events(&mut app, 22);
    assert_eq!(selected(&mut app, &route), 20);

    send(&mut app, &route, shifted('g'));
    assert_eq!(selected(&mut app, &route), 22);
    update_events(&mut app, 23);
    assert_eq!(selected(&mut app, &route), 23);
    send(&mut app, &route, TuiEvent::Key(KeyEvent::from(Key::Up)));
    send(&mut app, &route, TuiEvent::Key(KeyEvent::from(Key::Down)));
    update_events(&mut app, 24);
    assert_eq!(selected(&mut app, &route), 24);

    let (layout, _) = render(&mut app);
    let toggle = control(&layout, "shift+g");
    let toggle_route = EventRoute::new(toggle.path.clone());
    send(&mut app, &toggle_route, hotkey("shift+g"));
    update_events(&mut app, 25);
    assert_eq!(selected(&mut app, &route), 25);

    let (layout, _) = render(&mut app);
    let toggle = control(&layout, "shift+g");
    let feed = focus_target(&layout, crate::app::events::FOCUS);
    app.dispatch_focus(&feed, false, &mut tuicore::FocusCtx::default());
    app.dispatch_focus(&toggle, true, &mut tuicore::FocusCtx::default());
    send(
        &mut app,
        &toggle_route,
        TuiEvent::Key(KeyEvent::from(Key::Enter)),
    );
    assert!(render(&mut app).1.contains("○──  |G|"));
    update_events(&mut app, 26);
    focus_feed(&mut app);
    assert_eq!(selected(&mut app, &route), 25);
    let ctx = send(&mut app, &toggle_route, hotkey("shift+g"));
    assert_feed_focus(&ctx);
    assert_eq!(selected(&mut app, &route), 26);
}

#[test]
fn event_filters_select_the_newest_match_and_overview_clears_all_filters() {
    let mut app = events_app(20);
    let route = focus_feed(&mut app);
    send(&mut app, &route, TuiEvent::Key(KeyEvent::from(Key::Up)));
    app.pages_mut().filter_events("beta".into());
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    assert!(
        matches!(app.take_pending_focus_request(), Some(tuicore::FocusRequest::Target(id)) if id.as_str() == crate::app::events::FOCUS)
    );
    assert_eq!(selected(&mut app, &route), 19);
    assert!(render(&mut app).1.contains("──●  |G|"));

    let ctx = send(&mut app, &route, shifted('t'));
    assert_feed_focus(&ctx);
    assert!(render(&mut app).1.contains("No events handed over"));
    let (layout, _) = render(&mut app);
    let toggle = control(&layout, "shift+t");
    app.dispatch_focus(&toggle, true, &mut tuicore::FocusCtx::default());
    let ctx = send(
        &mut app,
        &EventRoute::new(toggle.path.clone()),
        shifted('h'),
    );
    assert_eq!(ctx.focus_request(), Some(&crate::app::initial_focus()));
    assert!(app.attached_sessions_only);
    select_tab(&mut app, 1);
    let route = focus_feed(&mut app);
    assert_eq!(selected(&mut app, &route), 20);
    let text = render(&mut app).1;
    assert!(
        text.contains("All providers") && text.contains("alpha") && text.contains("beta"),
        "{text}"
    );

    send(
        &mut app,
        &route,
        TuiEvent::Key(KeyEvent::from(Key::Char('/'))),
    );
    let (layout, _) = render(&mut app);
    let search = focus_target(&layout, "input");
    let search_route = EventRoute::new(search.path.clone());
    app.dispatch_focus(&search, true, &mut tuicore::FocusCtx::default());
    send(&mut app, &search_route, TuiEvent::Paste("event-019".into()));
    let ctx = send(&mut app, &search_route, hotkey("shift+h"));
    assert_eq!(ctx.focus_request(), Some(&crate::app::initial_focus()));
    select_tab(&mut app, 1);
    let route = focus_feed(&mut app);
    assert_eq!(selected(&mut app, &route), 20);
    assert!(render(&mut app).1.contains("──●  |G|"));
    update_events(&mut app, 21);
    assert_eq!(selected(&mut app, &route), 21);
    let text = render(&mut app).1;
    assert!(
        text.contains("event-021") && !text.contains("event-001"),
        "{text}"
    );
}

#[test]
fn tab_from_the_event_feed_focuses_the_provider_filter_directly() {
    let mut app = events_app(3);
    let (layout, _) = render(&mut app);
    let feed = focus_target(&layout, crate::app::events::FOCUS);
    let filter = control(&layout, "shift+p");
    let mut focus = tuicore::FocusManager::new();
    let mut dispatcher = tuicore::TreeDispatcher::new();
    let settings = AnimationSettings::default();
    let transition = focus
        .apply_request(
            &tuicore::FocusRequest::TargetAt {
                path: feed.path,
                id: feed.id,
            },
            layout.focus_targets(),
        )
        .unwrap();
    dispatcher.dispatch_focus(&mut app, transition, settings);
    let effects = dispatcher.dispatch_event(
        &mut app,
        &EventRoute::new(focus.current_path()),
        &TuiEvent::Key(KeyEvent::from(Key::Tab)),
        settings,
    );
    let transition = focus
        .apply_request(
            effects.focus_request.as_ref().unwrap(),
            layout.focus_targets(),
        )
        .unwrap();
    dispatcher.dispatch_focus(&mut app, transition, settings);
    assert_eq!(focus.current().unwrap().path, filter.path);
    assert_eq!(focus.current().unwrap().id, filter.id);
}

#[test]
fn escape_and_control_bracket_return_event_controls_to_the_feed() {
    let mut app = events_app(3);
    for sequence in ["shift+p", "shift+t", "shift+g"] {
        for event in [TuiEvent::Key(KeyEvent::from(Key::Esc)), control_bracket()] {
            let (layout, _) = render(&mut app);
            let target = control(&layout, sequence);
            let route = EventRoute::new(target.path.clone());
            app.dispatch_focus(&target, true, &mut tuicore::FocusCtx::default());
            let ctx = send(&mut app, &route, event);
            assert_feed_focus(&ctx);
        }
    }
    let route = focus_feed(&mut app);
    send(&mut app, &route, shifted('p'));
    let (layout, _) = render(&mut app);
    let search = focus_target(&layout, "input");
    let ctx = send(
        &mut app,
        &EventRoute::new(search.path.clone()),
        control_bracket(),
    );
    assert_feed_focus(&ctx);
    assert!(render(&mut app).0.overlays().is_empty());
}

#[test]
fn an_empty_event_feed_follows_its_first_arrival() {
    let mut app = events_app(0);
    let route = focus_feed(&mut app);
    update_events(&mut app, 1);
    assert_eq!(selected(&mut app, &route), 1);
    assert!(render(&mut app).1.contains("──●  |G|"));
}

#[test]
fn overview_resets_search_and_selection_on_all_tabs_and_returns_to_sessions() {
    init_ui();
    for tab in 0..4 {
        for event in [shifted('h'), hotkey("shift+h")] {
            for routed in [false, true] {
                let service = AppService::for_tests();
                service.set_opencode_snapshot_for_tests(super::attached_sessions::observation());
                let mut app = crate::app::root(service);
                app.update_snapshot(snapshot());
                app.pages_mut()
                    .update_providers(crate::store::providers::Snapshot {
                        providers: ["alpha", "beta"]
                            .map(|name| crate::store::providers::Provider {
                                name: name.into(),
                                directory: format!("/templates/providers/{name}"),
                                manifest: Some(crate::store::providers::Manifest {
                                    schema_version: 1,
                                    name: name.into(),
                                    profile: "message".into(),
                                    description: name.into(),
                                    protocol: "tandem-events-v1".into(),
                                    feedback: vec![],
                                }),
                                available: true,
                                status: crate::store::providers::Status::NotStarted,
                                container_id: None,
                                error: None,
                                operation: None,
                            })
                            .to_vec(),
                        error: None,
                    });
                update_events(&mut app, 20);
                for page in 0..4 {
                    select_tab(&mut app, page);
                    let (layout, _) = render(&mut app);
                    let list = focus_target(
                        &layout,
                        match page {
                            1 => crate::app::events::FOCUS,
                            2 => crate::app::providers::FOCUS,
                            _ => crate::app::TREE_FOCUS,
                        },
                    );
                    let route = EventRoute::new(list.path.clone());
                    app.dispatch_focus(&list, true, &mut tuicore::FocusCtx::default());
                    send(&mut app, &route, TuiEvent::Key(KeyEvent::from(Key::End)));
                    send(
                        &mut app,
                        &route,
                        TuiEvent::Key(KeyEvent::from(Key::Char('/'))),
                    );
                    let (layout, _) = render(&mut app);
                    let search = focus_target(&layout, "input");
                    let search_route = EventRoute::new(search.path.clone());
                    app.dispatch_focus(&search, true, &mut tuicore::FocusCtx::default());
                    send(
                        &mut app,
                        &search_route,
                        TuiEvent::Paste(
                            match page {
                                1 => "event-019",
                                2 => "beta",
                                _ => "missing",
                            }
                            .into(),
                        ),
                    );
                    if page != tab {
                        send(
                            &mut app,
                            &search_route,
                            TuiEvent::Key(KeyEvent::from(Key::Enter)),
                        );
                    }
                }
                select_tab(&mut app, tab);
                assert_eq!(app.tabs_mut().selected_index(), tab);
                let (layout, _) = render(&mut app);
                let search = focus_target(&layout, "input");
                let search_route = EventRoute::new(search.path.clone());
                app.dispatch_focus(&search, true, &mut tuicore::FocusCtx::default());
                let mut ctx = EventCtx::default();
                let outcome = if routed {
                    app.dispatch_event(&search_route, &event, &mut ctx)
                } else {
                    app.event(&event, &mut ctx)
                };
                assert_eq!(outcome, tuicore::EventOutcome::Handled);
                assert_eq!(ctx.focus_request(), Some(&crate::app::initial_focus()));
                assert!(app.attached_sessions_only);
                assert!(!app.events_active && !app.providers_active);
                assert!(app.running_only);
                assert!(!app.opencode_history);
                assert_eq!(app.tabs_mut().selected_index(), 0);
                let (layout, _) = render(&mut app);
                focus_target(&layout, crate::app::TREE_FOCUS);
                assert!(layout.overlays().is_empty());
                for page in 0..4 {
                    select_tab(&mut app, page);
                    let (layout, text) = render(&mut app);
                    assert!(layout.overlays().is_empty(), "tab {page}");
                    assert!(text.contains("Search… |/|"), "tab {page}: {text}");
                    let list = focus_target(
                        &layout,
                        match page {
                            1 => crate::app::events::FOCUS,
                            2 => crate::app::providers::FOCUS,
                            _ => crate::app::TREE_FOCUS,
                        },
                    );
                    app.dispatch_focus(&list, true, &mut tuicore::FocusCtx::default());
                    match page {
                        1 => {
                            assert_eq!(selected(&mut app, &EventRoute::new(list.path.clone())), 20);
                            assert!(
                                text.contains("event-020") && !text.contains("event-001"),
                                "{text}"
                            );
                            assert!(text.contains("──●  |G|"), "{text}");
                        }
                        2 => {
                            assert!(text.contains("alpha") && text.contains("beta"), "{text}");
                            let ctx = send(
                                &mut app,
                                &EventRoute::new(list.path.clone()),
                                TuiEvent::Key(KeyEvent::from(Key::Enter)),
                            );
                            assert!(
                                matches!(ctx.messages(), [Msg::ProviderDetails(provider)] if provider.name == "alpha")
                            );
                        }
                        _ => {
                            assert!(text.contains("review"), "{text}");
                            assert_eq!(
                                app.selected().unwrap().id,
                                if page == 0 {
                                    "instance:review"
                                } else {
                                    "template:/tmp/templates/website"
                                }
                            );
                            assert!(!crate::app::instances::is_searching(&app.instances));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn escape_clears_a_committed_event_search_and_resumes_the_full_feed() {
    let mut app = events_app(20);
    let route = focus_feed(&mut app);
    send(
        &mut app,
        &route,
        TuiEvent::Key(KeyEvent::from(Key::Char('/'))),
    );
    let (layout, _) = render(&mut app);
    let search = focus_target(&layout, "input");
    let search_route = EventRoute::new(search.path.clone());
    app.dispatch_focus(&search, true, &mut tuicore::FocusCtx::default());
    send(&mut app, &search_route, TuiEvent::Paste("event-019".into()));
    send(
        &mut app,
        &search_route,
        TuiEvent::Key(KeyEvent::from(Key::Enter)),
    );
    focus_feed(&mut app);
    assert_eq!(selected(&mut app, &route), 19);
    send(&mut app, &route, TuiEvent::Key(KeyEvent::from(Key::Esc)));
    assert_eq!(selected(&mut app, &route), 20);
    assert!(render(&mut app).1.contains("Search… |/|"));
}
