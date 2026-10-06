use super::*;
use crate::store::events::Batch;

fn setup(url: Option<&str>) -> (App, EventRoute) {
    init_ui();
    let service = AppService::for_tests();
    let token = service.register_provider_for_tests("sample");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut event = crate::environments::events::tests::event("linked-event");
    event.url = url.map(str::to_owned);
    runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![event],
            },
        ))
        .unwrap();
    let snapshot = runtime.block_on(service.list_events()).unwrap();
    assert_eq!(snapshot.records[0].event.url.as_deref(), url);
    let mut app = root(service);
    app.pages_mut().update_events(snapshot);
    app.handle_message(
        Msg::ProviderStreamEvents("sample".into(), "samples".into()),
        &mut EventCtx::default(),
    );
    super::events::show_all_events(&mut app);
    let (layout, _) = super::events::render(&mut app, 130);
    let feed = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    app.dispatch_focus(feed, true, &mut tuicore::FocusCtx::default());
    let route = EventRoute::new(feed.path.clone());
    (app, route)
}

fn press(app: &mut App, route: &EventRoute, key: Key) -> EventCtx<Msg> {
    let mut ctx = EventCtx::default();
    if app.menu_layer().is_active() || app.view.is_active() {
        app.event(&TuiEvent::Key(key.into()), &mut ctx);
    } else {
        app.dispatch_event(route, &TuiEvent::Key(key.into()), &mut ctx);
    }
    let messages: Vec<_> = ctx.drain_messages().collect();
    for message in messages {
        app.handle_message(message, &mut ctx);
    }
    ctx
}

fn activate(app: &mut App, route: &EventRoute, menu: bool) -> EventCtx<Msg> {
    if menu {
        press(app, route, Key::Char('.'));
        let (_, text) = super::events::render(app, 130);
        let line = text
            .lines()
            .find(|line| line.contains("Open link"))
            .unwrap();
        assert!(line.contains("Enter"), "{line}");
    }
    press(app, route, Key::Enter)
}

#[test]
fn event_enter_and_menu_open_the_published_link_with_the_system_handler() {
    for url in [
        "https://example.com/events/42?tab=details",
        "http://localhost:8080/events/42",
        "slack://channel?team=T123456&id=C123456",
        "mailto:support@example.com",
    ] {
        for menu in [false, true] {
            let (mut app, route) = setup(Some(url));
            let ctx = activate(&mut app, &route, menu);
            assert_eq!(app.service.opened_system_targets(), [url]);
            assert!(ctx.notifications().is_empty());
            assert!(!app.view.is_active());
            assert!(!app.menu_layer().is_active());
        }
    }
}

#[test]
fn event_enter_and_menu_warn_when_the_published_link_is_missing_or_blank() {
    for url in [None, Some(""), Some(" \t ")] {
        for menu in [false, true] {
            let (mut app, route) = setup(url);
            let ctx = activate(&mut app, &route, menu);
            let [notice] = ctx.notifications() else {
                panic!("Expected one missing-link warning");
            };
            assert_eq!(notice.kind(), tuicore::NotificationKind::Warning);
            assert_eq!(notice.title(), "No link set");
            assert_eq!(notice.body(), "This event has no link to open.");
            assert!(app.service.opened_system_targets().is_empty());
            assert!(!app.view.is_active());
            assert!(!app.menu_layer().is_active());
        }
    }
}

#[test]
fn event_links_require_valid_absolute_urls() {
    for url in ["not a URL", "https://", "/tmp/event.html", "--help"] {
        let (mut app, route) = setup(Some(url));
        let ctx = activate(&mut app, &route, false);
        let [notice] = ctx.notifications() else {
            panic!("Expected one invalid-link error");
        };
        assert_eq!(notice.kind(), tuicore::NotificationKind::Error);
        assert_eq!(notice.title(), "Cannot open link");
        assert_eq!(notice.body(), "Event link must be a valid absolute URL");
        assert!(app.service.opened_system_targets().is_empty());
        assert!(!app.view.is_active());
    }
}

#[test]
fn event_details_and_search_submission_preserve_their_distinct_actions() {
    let (mut app, route) = setup(Some("https://example.com/events/42"));
    press(&mut app, &route, Key::Char('d'));
    assert!(app.details_open);
    assert!(app.view.is_active());
    let (_, text) = super::events::render(&mut app, 130);
    assert!(text.contains("linked-event"), "{text}");
    assert!(app.service.opened_system_targets().is_empty());
    app.handle_message(Msg::Close, &mut EventCtx::default());
    let (layout, _) = super::events::render(&mut app, 130);
    let feed = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    app.dispatch_focus(feed, true, &mut tuicore::FocusCtx::default());
    press(&mut app, &route, Key::Char('/'));
    let (layout, _) = super::events::render(&mut app, 130);
    let search = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == "input")
        .unwrap();
    app.dispatch_focus(search, true, &mut tuicore::FocusCtx::default());
    let search_route = EventRoute::new(search.path.clone());
    for character in "sample".chars() {
        press(&mut app, &search_route, Key::Char(character));
    }
    let ctx = press(&mut app, &search_route, Key::Enter);
    assert!(ctx.notifications().is_empty());
    assert!(app.service.opened_system_targets().is_empty());
    assert!(!app.view.is_active());
}
