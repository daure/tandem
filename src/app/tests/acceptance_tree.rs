use super::events::render;
use super::*;
use crate::{
    app::{
        acceptances::{Context, Target},
        row_actions::Command,
    },
    store::{
        opencode::{Activity, Pane, Session},
        rules::AcceptanceWorkspace,
    },
};
use std::time::Duration;

fn acceptance() -> crate::store::rules::Acceptance {
    let mut acceptance = super::rules::acceptance(super::rules::rule("inspect"), 71, 7);
    acceptance.instance = "review".into();
    acceptance
}

fn conversation(id: &str, title: &str) -> Session {
    Session {
        id: id.into(),
        title: title.into(),
        directory: "/tmp/workspaces/review".into(),
        server: "http://127.0.0.1:12345".into(),
        activity: Activity::Idle,
        ..Default::default()
    }
}

#[test]
fn acceptance_conversation_picker_lists_retained_titles_and_original_identity() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    app.pages_mut()
        .acceptance_context()
        .borrow_mut()
        .inventory
        .observed_at_unix_seconds = Some(1);
    let mut acceptance = acceptance();
    acceptance.session_id = Some("ses_original".into());
    let workspace = AcceptanceWorkspace {
        directory: "/tmp/workspaces/review".into(),
        sessions: vec![
            conversation("ses_original", "Original task"),
            conversation("ses_followup", "Follow-up task"),
        ],
    };
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        acceptances: vec![acceptance.clone()],
        workspaces: [(acceptance.id, workspace.clone())].into(),
        ..Default::default()
    });
    let target = Target::new(&acceptance, Some(&workspace), &Context::default(), false);
    assert!(target.enabled(Command::Session));
    assert!(target.conversations.is_empty());
    app.acceptance_action(target, Command::Session, &mut EventCtx::default());
    let (_, text) = render(&mut app, 160);
    assert!(text.contains("Original task · original"), "{text}");
    assert!(text.contains("Follow-up task"), "{text}");
    app.handle_message(Msg::Close, &mut EventCtx::default());
    let mut target = Target::new(&acceptance, Some(&workspace), &Context::default(), true);
    target.selected = Some(target.conversations[0].clone());
    app.acceptance_action(target, Command::Session, &mut EventCtx::default());
    let (_, text) = render(&mut app, 160);
    assert!(
        text.contains("Recreate instance and reopen conversation"),
        "{text}"
    );
    assert!(
        text.contains("deleted files, uncommitted")
            && text.contains("changes and runtime data are not restored"),
        "{text}"
    );
    assert!(app.service.operations().is_empty());
}

#[test]
fn event_tree_keeps_exact_acceptance_and_conversation_actions_after_inventory_removal() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    let acceptance = acceptance();
    let saved = conversation("ses_original", "Original task");
    let mut attached = conversation("ses_active", "Active task");
    attached.panes = vec![Pane {
        session: "main".into(),
        id: 2,
        tab_id: 1,
        tab_name: "Inspect".into(),
    }];
    app.update_snapshot(snapshot());
    app.opencode_snapshot.sessions = vec![attached, saved.clone()];
    app.update_overview_rows(&snapshot(), &[], false);
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        acceptances: vec![acceptance.clone()],
        workspaces: [(
            acceptance.id,
            AcceptanceWorkspace {
                directory: saved.directory.clone(),
                sessions: vec![saved],
            },
        )]
        .into(),
        ..Default::default()
    });
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            records: vec![crate::store::events::Record {
                sequence: 7,
                provider: "sample".into(),
                received_at: "now".into(),
                event: crate::environments::events::tests::event("task"),
                attempts: vec![],
                acceptances: vec![acceptance.clone()],
            }],
            total: 1,
            ..Default::default()
        });
    app.tabs_mut().select_index(1);
    app.after_event(&mut EventCtx::default());
    app.pages_mut().tick(
        Duration::ZERO,
        AnimationSettings {
            enabled: false,
            ..Default::default()
        },
    );
    let (layout, _) = render(&mut app, 160);
    let focus = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
    let route = EventRoute::new(focus.path.clone());
    for key in [Key::Right, Key::Down] {
        app.dispatch_event(
            &route,
            &TuiEvent::Key(key.into()),
            &mut EventCtx::new(AnimationSettings {
                enabled: false,
                ..Default::default()
            }),
        );
    }
    let (_, text) = render(&mut app, 160);
    assert!(text.contains("inspect #71 · event #7"), "{text}");
    assert!(text.contains("review · Running · 󰠲 website"), "{text}");
    for (key, command) in [
        ('r', Command::Rule),
        ('v', Command::Instance),
        ('n', Command::NewSession),
        ('p', Command::PurgeInstance),
        ('o', Command::Session),
    ] {
        let mut ctx = EventCtx::default();
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char(key).into()), &mut ctx);
        assert!(
            matches!(ctx.messages(), [Msg::AcceptanceAction(target, action)] if target.acceptance.id == 71 && *action == command)
        );
    }
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &route,
        &TuiEvent::Key(KeyEvent {
            code: Key::Enter,
            modifiers: KeyModifiers::CONTROL,
        }),
        &mut ctx,
    );
    assert!(
        matches!(ctx.messages(), [Msg::AcceptanceAction(target, Command::Routes)] if target.acceptance.id == 71)
    );
    for key in [Key::Right, Key::Down] {
        app.dispatch_event(
            &route,
            &TuiEvent::Key(key.into()),
            &mut EventCtx::new(AnimationSettings {
                enabled: false,
                ..Default::default()
            }),
        );
    }
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('o').into()), &mut ctx);
    assert!(
        matches!(ctx.messages(), [Msg::AcceptanceAction(target, Command::Session)] if target.selected.as_ref().unwrap().label.contains("Active task"))
    );
    let (_, text) = render(&mut app, 160);
    assert!(text.contains("Active task"), "{text}");
    app.update_snapshot(EnvironmentSnapshot::default());
    app.opencode_snapshot = Default::default();
    app.toolbar_state.borrow_mut().show_saved = true;
    app.update_overview_rows(
        &EnvironmentSnapshot {
            observed_at_unix_seconds: Some(1),
            ..Default::default()
        },
        &[],
        false,
    );
    app.pages_mut().tick(
        Duration::ZERO,
        AnimationSettings {
            enabled: false,
            ..Default::default()
        },
    );
    let (_, text) = render(&mut app, 160);
    assert!(
        text.contains("instance deleted") && text.contains("Original task"),
        "{text}"
    );
}

#[test]
fn acceptance_shortcuts_open_its_routes_and_focus_its_rule_from_both_views() {
    init_ui();
    for history in [false, true] {
        let mut app = crate::app::root(AppService::for_tests());
        let acceptance = acceptance();
        let mut inventory = snapshot();
        inventory.instances[0].services.push(InstanceService {
            name: "api".into(),
            url: Some("http://localhost:9876/review/api/".into()),
            ..Default::default()
        });
        app.update_snapshot(inventory);
        app.pages_mut().update_rules(crate::store::rules::Snapshot {
            rules: vec![super::rules::rule("other"), acceptance.rule.clone()],
            acceptances: vec![acceptance.clone()],
            ..Default::default()
        });
        if history {
            app.open_rule(acceptance.rule.clone(), &mut EventCtx::default());
        } else {
            app.pages_mut()
                .update_events(crate::store::events::Snapshot {
                    records: vec![crate::store::events::Record {
                        sequence: 7,
                        provider: "sample".into(),
                        received_at: "now".into(),
                        event: crate::environments::events::tests::event("task"),
                        attempts: vec![],
                        acceptances: vec![acceptance.clone()],
                    }],
                    total: 1,
                    ..Default::default()
                });
            app.tabs_mut().select_index(1);
            app.after_event(&mut EventCtx::default());
        }
        let focus_id = if history {
            "acceptance-list"
        } else {
            crate::app::events::FOCUS
        };
        app.pages_mut().tick(
            Duration::ZERO,
            AnimationSettings {
                enabled: false,
                ..Default::default()
            },
        );
        let (layout, _) = render(&mut app, 160);
        let focus = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == focus_id)
            .unwrap();
        app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
        let route = EventRoute::new(focus.path.clone());
        if !history {
            for key in [Key::Right, Key::Down] {
                app.dispatch_event(
                    &route,
                    &TuiEvent::Key(key.into()),
                    &mut EventCtx::new(AnimationSettings {
                        enabled: false,
                        ..Default::default()
                    }),
                );
            }
        }
        let mut ctx = EventCtx::default();
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent {
                code: Key::Enter,
                modifiers: KeyModifiers::CONTROL,
            }),
            &mut ctx,
        );
        assert!(matches!(
            ctx.messages(),
            [Msg::AcceptanceAction(_, Command::Routes)]
        ));
        for message in ctx.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
        assert!(app.route_layer().is_active());
        let (_, text) = render(&mut app, 160);
        assert!(
            text.contains("api - http://localhost:9876/review/api/"),
            "{text}"
        );
        assert!(
            text.contains("web - http://localhost:9876/review/web/"),
            "{text}"
        );
        app.event(&TuiEvent::Key(Key::Enter.into()), &mut EventCtx::default());
        assert_eq!(
            app.service.opened_system_targets(),
            ["http://localhost:9876/review/web/"]
        );

        if history {
            app.open_rule(acceptance.rule.clone(), &mut EventCtx::default());
        }
        let (layout, _) = render(&mut app, 160);
        let focus = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == focus_id)
            .unwrap();
        app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
        let mut ctx = EventCtx::default();
        app.dispatch_event(
            &EventRoute::new(focus.path.clone()),
            &TuiEvent::Key(Key::Char('r').into()),
            &mut ctx,
        );
        assert!(matches!(
            ctx.messages(),
            [Msg::AcceptanceAction(_, Command::Rule)]
        ));
        for message in ctx.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
        assert!(!app.view.is_active());
        assert!(app.rules_active);
        app.pages_mut().tick(
            Duration::ZERO,
            AnimationSettings {
                enabled: false,
                ..Default::default()
            },
        );
        let (layout, _) = render(&mut app, 160);
        let focus = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == crate::app::rules::FOCUS)
            .unwrap();
        app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
        let mut ctx = EventCtx::default();
        app.dispatch_event(
            &EventRoute::new(focus.path.clone()),
            &TuiEvent::Key(Key::Enter.into()),
            &mut ctx,
        );
        assert!(
            matches!(ctx.messages(), [Msg::OpenRule(rule)] if rule.definition.name == "inspect")
        );
    }
}
