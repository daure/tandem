use super::*;
use crate::store::events::{Batch, Payload};
use std::time::{Duration, Instant};

fn refresh_events(app: &mut App, count: u64) {
    refresh_events_until(app, |snapshot| snapshot.total == count);
}

fn refresh_events_until(app: &mut App, ready: impl Fn(&crate::store::events::Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.service.poll_events();
        app.tick(
            Duration::ZERO,
            AnimationSettings {
                enabled: false,
                ..Default::default()
            },
        );
        if ready(&app.service.event_snapshot()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "event observer did not publish its snapshot"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    app.tick(
        Duration::ZERO,
        AnimationSettings {
            enabled: false,
            ..Default::default()
        },
    );
}

fn render(app: &mut App, width: u16) -> (tuicore::LayoutCtx, String) {
    let area = Rect::new(0, 0, width, 30);
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

#[test]
fn events_are_the_second_tab_and_render_all_four_profiles_with_inspectable_metadata() {
    init_ui();
    let service = AppService::for_tests();
    let token = service.register_provider_for_tests("sample");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let payloads = [
        serde_json::json!({"profile":"message","data":{"author":"Alex","channel":"development","thread":"thread-1","text":"A thread reply"}}),
        serde_json::json!({"profile":"ticket","data":{"key":"DEV-42","title":"Build event ingestion","status":"In progress","assignee":"Sam"}}),
        serde_json::json!({"profile":"system_event","data":{"resource":"api-server","environment":"prod","signal":"health.failed","severity":"Error","description":"The build failed"}}),
        serde_json::json!({"profile":"generic","data":{"counter":42}}),
    ];
    let events = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            let mut event = crate::environments::events::tests::event(&format!("event-{index}"));
            event.payload = serde_json::from_value::<Payload>(payload).unwrap();
            if index == 3 {
                event.event_type = "sample.generic.observed".into();
                event.summary = "A sensor sample".into();
            }
            event
        })
        .collect();
    runtime
        .block_on(service.ingest_events(token.clone(), Batch { events }))
        .unwrap();
    let mut app = crate::app::root(service.clone());
    app.update_snapshot(snapshot());
    refresh_events(&mut app, 4);
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
        &mut EventCtx::default(),
    );
    assert_eq!(app.tabs_mut().selected_index(), 1);
    assert!(app.events_active);
    let theme = tuicore::theme();
    for row in service.event_snapshot().records {
        let text = crate::app::events::row_text(&row);
        assert_eq!(text.lines.len(), 2);
        assert_eq!(text.lines[0].spans[0].style.fg, Some(theme.text_fg()));
        assert_eq!(text.lines[0].spans[2].style.fg, Some(theme.text_fg()));
        assert!(
            text.lines[0].spans[2]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
        assert_eq!(text.lines[1].spans[0].style.fg, Some(theme.text_fg()));
        for span in &text.lines[0].spans {
            let color = match span.content.as_ref() {
                " · " | "󱡠" | "Sam" | "production" | "sample.generic.observed" => {
                    theme.muted_fg()
                }
                "Error" => theme.error_fg(),
                _ => theme.text_fg(),
            };
            assert_eq!(span.style.fg, Some(color), "{}", span.content);
        }
        let mut accepted = row.clone();
        accepted.attempts[0].status = crate::store::events::ProcessingStatus::Accepted;
        let accepted_text = crate::app::events::row_text(&accepted);
        assert_eq!(accepted_text.lines.len(), 2);
        assert_eq!(
            accepted_text.lines[0].spans[0].style.fg,
            Some(theme.success_fg())
        );
        assert_eq!(accepted_text.lines[0].spans[1..], text.lines[0].spans[1..]);
        assert_eq!(accepted_text.lines[1], text.lines[1]);
        accepted.attempts.insert(0, row.attempts[0].clone());
        assert_eq!(crate::app::events::row_text(&accepted), text);
        accepted.attempts.clear();
        assert_eq!(crate::app::events::row_text(&accepted), text);
    }
    for width in [40, 130] {
        let (layout, text) = render(&mut app, width);
        assert!(text.contains("Events"), "{text}");
        for label in [" sample", " sample", " sample", " sample"] {
            assert!(text.contains(label), "{text}");
        }
        if width == 130 {
            let lines: Vec<_> = text.lines().map(str::trim).collect();
            for (header, body) in [
                (" sample · Alex · development · 󱡠", "A thread reply"),
                (
                    " sample · DEV-42 · In progress · Sam",
                    "Build event ingestion",
                ),
                (
                    " sample · api-server · production · health.failed · Error",
                    "The build failed",
                ),
                (" sample · sample.generic.observed", "A sensor sample"),
            ] {
                let index = lines.iter().position(|line| *line == header).unwrap();
                assert_eq!(lines[index + 1], body);
            }
        }
        assert!(
            layout
                .focus_targets()
                .iter()
                .any(|target| target.enabled && target.id.as_str() == crate::app::events::FOCUS)
        );
    }
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('p'))),
        &mut EventCtx::default(),
    );
    assert!(app.intent.is_none());
    let (layout, _) = render(&mut app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    let route = EventRoute::new(target.path.clone());
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let mut menu = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Char('.'))),
        &mut menu,
    );
    for message in menu.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    assert!(app.menu_layer().is_active());
    let text = render(&mut app, 130).1;
    for (label, key) in [
        ("View details", "Enter"),
        ("Replay", "r"),
        ("Go to provider", "p"),
    ] {
        let line = text
            .lines()
            .find(|line| line.contains(&format!("{label}   ")))
            .unwrap();
        assert!(line.trim_end_matches([' ', '┃']).ends_with(key), "{line}");
    }
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Esc)),
        &mut EventCtx::default(),
    );
    assert!(!app.menu_layer().is_active());
    let mut activation = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut activation,
    );
    let row = match activation.messages() {
        [Msg::OpenEvent(row)] => row.clone(),
        other => panic!("Unexpected messages: {other:?}"),
    };
    assert_eq!(row.event.event_id, "event-3");
    let event_id = row.event.event_id.clone();
    app.handle_message(Msg::OpenEvent(row), &mut EventCtx::default());
    let text = render(&mut app, 130).1;
    assert!(
        text.contains(&format!("\"event_id\": \"{event_id}\"")),
        "{text}"
    );
    app.handle_message(Msg::Close, &mut EventCtx::default());
    runtime
        .block_on(service.ingest_events(
            token.clone(),
            Batch {
                events: vec![crate::environments::events::tests::event("later")],
            },
        ))
        .unwrap();
    refresh_events(&mut app, 5);
    let mut activation = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut activation,
    );
    assert!(
        matches!(activation.messages(), [Msg::OpenEvent(row)] if row.event.event_id == "later")
    );
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Up)),
        &mut EventCtx::default(),
    );
    let mut activation = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut activation,
    );
    let selected = match activation.messages() {
        [Msg::OpenEvent(row)] => row.sequence,
        other => panic!("Unexpected messages: {other:?}"),
    };
    runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("newest")],
            },
        ))
        .unwrap();
    refresh_events(&mut app, 6);
    let mut activation = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent::from(Key::Enter)),
        &mut activation,
    );
    assert!(matches!(activation.messages(), [Msg::OpenEvent(row)] if row.sequence == selected));
}

fn record(provider: &str, payload: serde_json::Value) -> crate::store::events::Record {
    let mut event = crate::environments::events::tests::event("presentation");
    event.payload = serde_json::from_value(payload).unwrap();
    crate::store::events::Record {
        sequence: 1,
        provider: provider.into(),
        received_at: "now".into(),
        event,
        attempts: vec![],
    }
}

#[test]
fn optional_event_fields_omit_blank_values_and_environment_aliases_are_display_only() {
    use serde_json::json;

    init_ui();
    for optional in [serde_json::Value::Null, json!(" \t "), json!("\u{202e}")] {
        let message = record(
            "slack-work",
            json!({"profile":"message","data":{"author":"Alex","channel":"development","text":"Hello","thread":optional}}),
        );
        assert_eq!(
            crate::app::events::row_text(&message).lines[0].to_string(),
            " slack-work · Alex · development"
        );
        let ticket = record(
            "jira-work",
            json!({"profile":"ticket","data":{"key":"DEV-42","status":"Open","title":"Fix","assignee":optional}}),
        );
        assert_eq!(
            crate::app::events::row_text(&ticket).lines[0].to_string(),
            " jira-work · DEV-42 · Open"
        );
        let system = record(
            "build-monitor",
            json!({"profile":"system_event","data":{"resource":"api-server","signal":"health.failed","severity":"odd label","description":"Check","environment":optional}}),
        );
        assert_eq!(
            crate::app::events::row_text(&system).lines[0].to_string(),
            " build-monitor · api-server · health.failed · odd label"
        );
        assert_eq!(
            crate::app::events::row_text(&system).lines[0]
                .spans
                .last()
                .unwrap()
                .style
                .fg,
            Some(tuicore::theme().text_fg())
        );
    }
    let unknown = record(
        "jira-work",
        json!({"profile":"ticket","data":{"key":"DEV-42","status":"Open","title":"Fix","assignee":" Unknown "}}),
    );
    assert_eq!(
        crate::app::events::row_text(&unknown).lines[0].to_string(),
        " jira-work · DEV-42 · Open · Unknown"
    );
    for (value, label) in [
        (" PROD ", "production"),
        ("Production", "production"),
        ("dev", "development"),
        ("DEVELOPMENT", "development"),
        ("stage", "staging"),
        ("Staging", "staging"),
        (" qa ", "qa"),
        ("sandbox", "sandbox"),
        (" preview-42 ", "preview-42"),
    ] {
        let system = record(
            "build-monitor",
            json!({"profile":"system_event","data":{"resource":"api-server","signal":"health.failed","severity":"Info","description":"Check","environment":value}}),
        );
        let original = system.clone();
        assert_eq!(
            crate::app::events::row_text(&system).lines[0].to_string(),
            format!(" build-monitor · api-server · {label} · health.failed · Info")
        );
        assert_eq!(system, original);
    }
}

#[test]
fn severity_override_colors_only_the_supplied_label_and_event_text_is_sanitized() {
    use serde_json::json;

    init_ui();
    let theme = tuicore::theme();
    for (color, expected) in [
        ("plain", theme.text_fg()),
        ("info", theme.info_fg()),
        ("warning", theme.warning_fg()),
        ("error", theme.error_fg()),
        ("success", theme.success_fg()),
    ] {
        let row = record(
            "monitor\u{202e}\n",
            json!({"profile":"system_event","data":{"resource":"api\tserver","environment":" qa\u{2066} ","signal":"health\u{202a}.failed","severity":" cUsToM\u{2069}label ","severity_color":color,"description":"Check\nnow\u{001b}"}}),
        );
        let text = crate::app::events::row_text(&row);
        assert_eq!(text.lines.len(), 2);
        assert_eq!(
            text.lines[0].to_string(),
            " monitor   · api server · qa · health .failed · cUsToM label"
        );
        assert_eq!(text.lines[1].to_string(), "Check now ");
        assert_eq!(text.lines[0].spans.last().unwrap().style.fg, Some(expected));
        assert_eq!(text.lines[0].spans[0].style.fg, Some(theme.text_fg()));
        assert_eq!(text.lines[0].spans[2].style.fg, Some(theme.text_fg()));
        assert_eq!(text.lines[1].spans[0].style.fg, Some(theme.text_fg()));
    }
    let message = record(
        "slack\u{202e}",
        json!({"profile":"message","data":{"author":"Alex\u{2066}","channel":"dev\nroom","text":"Hi\tthere","thread":"\u{202e}reply"}}),
    );
    let text = crate::app::events::row_text(&message);
    assert_eq!(text.lines[0].to_string(), " slack  · Alex  · dev room · 󱡠");
    assert_eq!(text.lines[1].to_string(), "Hi there");
    let ticket = record(
        "jira",
        json!({"profile":"ticket","data":{"key":"DEV\u{2069}-42","status":"In\rprogress","title":"Fix\nnow","assignee":" Sa\u{202e}m "}}),
    );
    let text = crate::app::events::row_text(&ticket);
    assert_eq!(
        text.lines[0].to_string(),
        " jira · DEV -42 · In progress · Sa m"
    );
    assert_eq!(text.lines[1].to_string(), "Fix now");
    let mut generic = record("sensor", json!({"profile":"generic","data":{}}));
    generic.event.event_type = "sample\u{202a}.observed".into();
    generic.event.summary = "A\nsummary".into();
    let text = crate::app::events::row_text(&generic);
    assert_eq!(text.lines[0].to_string(), " sensor · sample .observed");
    assert_eq!(text.lines[1].to_string(), "A summary");
}

#[test]
fn provider_dropdown_fits_its_label_and_popup_rows_within_terminal_bounds() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    let providers = ["dev-system-event", "production-system-event-provider"];
    let records = providers
        .into_iter()
        .enumerate()
        .map(|(index, provider)| crate::store::events::Record {
            sequence: index as i64,
            provider: provider.into(),
            received_at: "now".into(),
            event: crate::environments::events::tests::event(&format!("event-{index}")),
            attempts: vec![],
        })
        .collect();
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            records,
            total: 2,
            ..Default::default()
        });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
        &mut EventCtx::default(),
    );
    let (layout, _) = render(&mut app, 130);
    let field = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target.id.as_str() == "field"
                && target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "events-page")
        })
        .unwrap();
    assert_eq!(field.area.width, 20);
    app.dispatch_event(
        &EventRoute::new(field.path.clone()),
        &TuiEvent::Key(KeyEvent {
            code: Key::Char('P'),
            modifiers: KeyModifiers::SHIFT,
        }),
        &mut EventCtx::default(),
    );
    for width in [130, 40, 30] {
        let (layout, text) = render(&mut app, width);
        let popup = layout.overlays().last().unwrap();
        assert_eq!(popup.area.width, (providers[1].len() as u16 + 2).min(width));
        assert!(popup.area.right() <= width);
        if width >= providers[1].len() as u16 + 2 {
            for provider in providers {
                assert!(text.contains(provider), "{text}");
            }
        }
    }
}

#[test]
fn provider_multiselect_filters_exact_names_preserves_refresh_and_accepts_single_provider_links() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    let records: Vec<_> = ["alpha", "alpha-other", "beta"]
        .into_iter()
        .enumerate()
        .map(|(index, provider)| crate::store::events::Record {
            sequence: index as i64,
            provider: provider.into(),
            received_at: "now".into(),
            event: crate::environments::events::tests::event(&format!("event-{index}")),
            attempts: vec![],
        })
        .collect();
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            records: records.clone(),
            total: 3,
            provider_totals: Default::default(),
            error: None,
        });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
        &mut EventCtx::default(),
    );
    let (layout, text) = render(&mut app, 130);
    assert!(text.contains("All providers"), "{text}");
    let feed = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    let feed_route = EventRoute::new(feed.path.clone());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &feed_route,
        &TuiEvent::Key(KeyEvent {
            code: Key::Char('P'),
            modifiers: tuicore::KeyModifiers::SHIFT,
        }),
        &mut ctx,
    );
    assert!(
        matches!(ctx.focus_request(), Some(tuicore::FocusRequest::TargetAt { id, .. }) if id.as_str() == "input")
    );
    let (layout, _) = render(&mut app, 130);
    let filter = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target.id.as_str() == "input"
                && target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "events-page")
        })
        .unwrap();
    let filter_route = EventRoute::new(filter.path.clone());
    app.dispatch_focus(filter, true, &mut tuicore::FocusCtx::default());
    let next = KeyEvent {
        code: Key::Char('j'),
        modifiers: tuicore::KeyModifiers::CONTROL,
    };
    for key in [
        KeyEvent::from(Key::Enter),
        next,
        next,
        KeyEvent::from(Key::Enter),
        KeyEvent {
            code: Key::Enter,
            modifiers: tuicore::KeyModifiers::CONTROL,
        },
    ] {
        let mut ctx = EventCtx::default();
        app.dispatch_event(&filter_route, &TuiEvent::Key(key), &mut ctx);
        assert!(ctx.messages().is_empty(), "{:?}", ctx.messages());
        render(&mut app, 130);
    }
    let text = render(&mut app, 130).1;
    assert!(
        text.contains(" alpha · Alex") && text.contains(" beta · Alex"),
        "{text}"
    );
    assert!(!text.contains(" alpha-other · Alex"), "{text}");
    let mut refreshed = records.clone();
    let mut later = records[0].clone();
    later.sequence = 4;
    later.event.event_id = "later".into();
    if let Payload::Message(message) = &mut later.event.payload {
        message.text = "A later arrival".into();
    }
    refreshed.push(later);
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            records: refreshed,
            total: 4,
            provider_totals: Default::default(),
            error: None,
        });
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let text = render(&mut app, 130).1;
    assert!(
        text.contains("A later arrival") && text.contains(" beta · Alex"),
        "{text}"
    );
    assert!(!text.contains(" alpha-other · Alex"), "{text}");
    app.handle_message(
        Msg::ProviderEvents("alpha".into()),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let text = render(&mut app, 130).1;
    assert!(text.contains(" alpha · Alex"), "{text}");
    assert!(
        !text.contains(" beta · Alex") && !text.contains(" alpha-other · Alex"),
        "{text}"
    );
}

#[test]
fn events_navigation_repairs_focus_and_remains_available_without_opencode() {
    init_ui();
    for enabled in [true, false] {
        let service = AppService::for_tests();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(service.set_opencode_enabled(enabled).unwrap())
            .unwrap()
            .unwrap();
        let mut app = crate::app::root(service);
        app.update_snapshot(snapshot());
        let (layout, _) = render(&mut app, 130);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == crate::app::TREE_FOCUS)
            .unwrap();
        let mut ctx = EventCtx::default();
        app.dispatch_event(
            &EventRoute::new(target.path.clone()),
            &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
            &mut ctx,
        );
        assert!(app.events_active);
        assert!(
            matches!(ctx.focus_request(), Some(tuicore::FocusRequest::Target(id)) if id.as_str() == crate::app::events::FOCUS)
        );
        let (layout, _) = render(&mut app, 130);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == crate::app::events::FOCUS)
            .unwrap();
        app.dispatch_event(
            &EventRoute::new(target.path.clone()),
            &TuiEvent::Key(KeyEvent::from(Key::Char(']'))),
            &mut EventCtx::default(),
        );
        assert!(!app.events_active);
    }
}

#[test]
fn handover_toggle_requires_assignment_and_preserves_provider_filter_on_refresh() {
    init_ui();
    let mut app = root(AppService::for_tests());
    let record = crate::store::events::Record {
        sequence: 1,
        provider: "alpha".into(),
        received_at: "now".into(),
        event: crate::environments::events::tests::event("accepted-event"),
        attempts: vec![crate::store::events::Attempt {
            id: 1,
            status: crate::store::events::ProcessingStatus::Accepted,
            replay: false,
            created_at: "now".into(),
            accepted_at: Some("now".into()),
        }],
    };
    let mut other = record.clone();
    other.sequence = 2;
    other.provider = "beta".into();
    other.event.event_id = "other-event".into();
    let mut snapshot = crate::store::events::Snapshot {
        records: vec![record, other],
        total: 662,
        ..Default::default()
    };
    app.pages_mut().update_events(snapshot.clone());
    app.handle_message(
        Msg::ProviderEvents("alpha".into()),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    for width in [40, 130] {
        let (layout, text) = render(&mut app, width);
        let header = text
            .lines()
            .find(|line| line.contains("1 of 662 events"))
            .unwrap();
        assert!(header.trim_end().ends_with("1 of 662 events"), "{text}");
        assert_eq!(
            header.trim_end().chars().count(),
            usize::from(width),
            "{text}"
        );
        assert!(header.contains(""), "{text}");
        let feed = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == crate::app::events::FOCUS)
            .unwrap();
        let route = EventRoute::new(feed.path.clone());
        let toggle = TuiEvent::Key(KeyEvent {
            code: Key::Char('T'),
            modifiers: tuicore::KeyModifiers::SHIFT,
        });
        let mut ctx = EventCtx::default();
        app.dispatch_event(&route, &toggle, &mut ctx);
        assert!(ctx.messages().is_empty());
        let (_, text) = render(&mut app, width);
        assert!(text.contains("0 of 662 events"), "{text}");
        assert!(text.contains("No events handed over"), "{text}");
        snapshot.total += 1;
        app.pages_mut().update_events(snapshot.clone());
        app.pages_mut()
            .tick(Duration::ZERO, AnimationSettings::default());
        assert!(
            render(&mut app, width)
                .1
                .contains(&format!("0 of {} events", snapshot.total))
        );
        app.dispatch_event(&route, &toggle, &mut EventCtx::default());
        let (_, text) = render(&mut app, width);
        assert!(text.contains(" alpha · Alex"), "{text}");
        assert!(!text.contains(" beta · Alex"), "{text}");
        snapshot.total = 662;
        app.pages_mut().update_events(snapshot.clone());
        app.pages_mut()
            .tick(Duration::ZERO, AnimationSettings::default());
    }
}
