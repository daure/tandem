use super::events::{refresh_events, render, show_all_events};
use super::*;
use crate::store::events::{Batch, Deletion, Record, Snapshot};
use std::time::Duration;

fn send(app: &mut App, route: Option<&EventRoute>, event: TuiEvent) {
    let mut ctx = EventCtx::default();
    if let Some(route) = route {
        app.dispatch_event(route, &event, &mut ctx);
    } else {
        app.event(&event, &mut ctx);
    }
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
}

#[test]
fn ignored_event_deletion_requires_approval_and_works_outside_the_feed_filters() {
    init_ui();
    let service = AppService::for_tests();
    let token = service.register_provider_for_tests("sample");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("ignored")],
            },
        ))
        .unwrap();
    let mut app = root(service.clone());
    refresh_events(&mut app, 1);
    app.handle_message(
        Msg::ProviderStreamEvents("missing".into(), "samples".into()),
        &mut EventCtx::default(),
    );
    for (width, approve) in [(130, false), (40, true)] {
        let (layout, text) = render(&mut app, width);
        let label = if width == 130 {
            "Delete ignored events |I|"
        } else {
            " I"
        };
        assert!(text.contains(label), "{text}");
        let target = layout
            .focus_targets()
            .iter()
            .filter(|target| {
                target.id.as_str()
                    == if approve {
                        crate::app::events::FOCUS
                    } else {
                        "button"
                    }
            })
            .max_by_key(|target| target.area.x)
            .unwrap();
        app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
        send(
            &mut app,
            Some(&EventRoute::new(target.path.clone())),
            TuiEvent::Key(if approve {
                KeyEvent {
                    code: Key::Char('I'),
                    modifiers: KeyModifiers::SHIFT,
                }
            } else {
                Key::Enter.into()
            }),
        );
        assert!(app.view.is_active());
        assert!(
            render(&mut app, 130)
                .1
                .contains("Delete all events with no acceptances")
        );
        assert!(app.event_action.is_none());
        assert_eq!(runtime.block_on(service.list_events()).unwrap().total, 1);
        send(
            &mut app,
            None,
            TuiEvent::Key(Key::Char(if approve { 'o' } else { 'c' }).into()),
        );
        assert!(!app.view.is_active());
        if approve {
            refresh_events(&mut app, 0);
        } else {
            assert!(app.event_action.is_none());
        }
    }
}

#[test]
fn acceptance_menus_delete_the_selected_acceptance_from_events_and_rule_history() {
    init_ui();
    for history in [false, true] {
        let mut app = root(AppService::for_tests());
        let acceptance = super::rules::acceptance(super::rules::rule("inspect"), 71, 7);
        app.pages_mut().update_rules(crate::store::rules::Snapshot {
            acceptances: vec![acceptance.clone()],
            ..Default::default()
        });
        if history {
            app.open_rule(acceptance.rule.clone(), &mut EventCtx::default());
        } else {
            app.pages_mut().update_events(Snapshot {
                records: vec![Record {
                    sequence: 7,
                    provider: "sample".into(),
                    received_at: "now".into(),
                    event: crate::environments::events::tests::event("accepted"),
                    attempts: vec![],
                    acceptances: vec![acceptance.clone()],
                }],
                total: 1,
                ..Default::default()
            });
            app.tabs_mut().select_index(1);
            app.after_event(&mut EventCtx::default());
        }
        let (layout, _) = render(&mut app, 160);
        let focus = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target.id.as_str()
                    == if history {
                        "acceptance-list"
                    } else {
                        crate::app::events::FOCUS
                    }
            })
            .unwrap();
        app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
        let route = EventRoute::new(focus.path.clone());
        if !history {
            send(&mut app, Some(&route), TuiEvent::Key(Key::Down.into()));
        }
        send(&mut app, Some(&route), TuiEvent::Key(Key::Char('.').into()));
        let (layout, text) = render(&mut app, 160);
        assert!(text.contains("Delete"), "{text}");
        let search = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == "input")
            .unwrap();
        send(
            &mut app,
            Some(&EventRoute::new(search.path.clone())),
            TuiEvent::Key(Key::Char('x').into()),
        );
        let text = render(&mut app, 160).1;
        assert!(text.contains("Delete acceptance"), "{text}");
        assert!(
            text.contains("Its event, instance and OpenCode sessions")
                && text.contains("are preserved."),
            "{text}"
        );
        assert!(app.event_action.is_none());
        send(&mut app, None, TuiEvent::Key(Key::Char('c').into()));
        assert!(!app.view.is_active());
        assert!(app.event_action.is_none());
    }
}

#[test]
fn acceptance_deletion_updates_pinned_events_outside_the_bounded_feed() {
    let mut app = root(AppService::for_tests());
    show_all_events(&mut app);
    let acceptance = super::rules::acceptance(super::rules::rule("inspect"), 71, 7);
    app.pages_mut().focus_event(Record {
        sequence: 7,
        provider: "sample".into(),
        received_at: "now".into(),
        event: crate::environments::events::tests::event("pinned"),
        attempts: vec![],
        acceptances: vec![acceptance],
    });
    app.tabs_mut().select_index(1);
    app.after_event(&mut EventCtx::default());
    assert!(render(&mut app, 130).1.contains("inspect #71"));
    app.pages_mut().forget_event(Deletion::Acceptance(71));
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let text = render(&mut app, 130).1;
    assert!(text.contains(" sample · #7"), "{text}");
    assert!(!text.contains("inspect #71"), "{text}");
    app.pages_mut().forget_event(Deletion::Ignored);
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    assert!(!render(&mut app, 130).1.contains(" sample · #7"));
}
