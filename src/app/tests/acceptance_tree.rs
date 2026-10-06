use super::events::render;
use super::*;
use crate::{
    app::{
        acceptances::{Context, Target},
        row_actions::Command,
    },
    store::{
        environments::UsageSummary,
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
fn event_session_creation_expands_its_acceptance_and_preserves_selection() {
    init_ui();
    for observed_first in [false, true] {
        let mut app = crate::app::root(AppService::for_tests());
        let acceptance = acceptance();
        let pane = Pane {
            session: "main".into(),
            id: 7,
            tab_id: 1,
            tab_name: "Review".into(),
        };
        let mut observation = crate::store::opencode::Snapshot {
            sessions: ["ses_first", "ses_last"]
                .map(|id| {
                    let mut session = conversation(id, id);
                    session.panes = vec![pane.clone()];
                    session
                })
                .to_vec(),
            ..Default::default()
        };
        app.service
            .set_opencode_snapshot_for_tests(observation.clone());
        app.update_snapshot(snapshot());
        app.pages_mut().update_rules(crate::store::rules::Snapshot {
            acceptances: vec![acceptance.clone()],
            ..Default::default()
        });
        let events = crate::store::events::Snapshot {
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
        };
        app.pages_mut().update_events(events.clone());
        app.tabs_mut().select_index(1);
        app.after_event(&mut EventCtx::default());
        let inventory_selection = app.selected().map(|row| row.id);
        let (layout, text) = render(&mut app, 160);
        assert!(!text.contains("ses_first"), "{text}");
        let focus = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == crate::app::events::FOCUS)
            .unwrap();
        app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
        let route = EventRoute::new(focus.path.clone());
        for key in [Key::Right, Key::Down] {
            app.dispatch_event(&route, &TuiEvent::Key(key.into()), &mut EventCtx::default());
        }
        let (sender, reply) = tokio::sync::oneshot::channel();
        app.opencode_action = Some(crate::app::opencode::PendingAction::creation(
            reply,
            crate::app::opencode::CreationView::Events {
                state: app.pages_mut().event_focus(),
                acceptance: acceptance.id,
            },
        ));
        let mut created = conversation("ses_new", "Created conversation");
        created.panes = vec![pane.clone()];
        observation.sessions.insert(1, created);
        if observed_first {
            app.service
                .set_opencode_snapshot_for_tests(observation.clone());
            app.update_snapshot(snapshot());
            render(&mut app, 160);
        }
        sender
            .send(Ok(crate::store::opencode::SessionLocation {
                session_id: Some("ses_new".into()),
                pane,
            }))
            .unwrap();
        app.poll_opencode_action();
        render(&mut app, 160);
        if !observed_first {
            let mut ctx = EventCtx::default();
            app.dispatch_event(&route, &TuiEvent::Key(Key::Char('d').into()), &mut ctx);
            assert!(matches!(ctx.messages(), [Msg::FocusRule(name)] if name == "inspect"));
            app.service.set_opencode_snapshot_for_tests(observation);
            app.update_snapshot(snapshot());
        }
        let (_, text) = render(&mut app, 160);
        assert!(text.contains("Created conversation"), "{text}");
        assert_eq!(app.selected().map(|row| row.id), inventory_selection);
        let mut updated = events;
        let mut newest = updated.records[0].clone();
        newest.sequence = 8;
        newest.acceptances.clear();
        updated.records.insert(0, newest);
        updated.total = 2;
        app.pages_mut().update_events(updated);
        render(&mut app, 160);
        let mut ctx = EventCtx::default();
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char('d').into()), &mut ctx);
        assert!(matches!(ctx.messages(), [Msg::FocusRule(name)] if name == "inspect"));
    }
}

#[test]
fn confirmed_event_instance_purge_returns_focus_to_the_event_tree() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    app.update_snapshot(snapshot());
    app.tabs_mut().select_index(1);
    app.after_event(&mut EventCtx::default());
    let target = Target::new(
        &acceptance(),
        None,
        &app.pages_mut().acceptance_context().borrow(),
        false,
    );
    app.acceptance_action(target, Command::PurgeInstance, &mut EventCtx::default());
    assert!(app.view.is_active());

    let mut ctx = EventCtx::default();
    app.handle_message(Msg::Submit, &mut ctx);

    assert!(!app.view.is_active());
    assert!(app.events_active);
    assert!(
        matches!(ctx.focus_request(), Some(tuicore::FocusRequest::Target(id)) if id.as_str() == crate::app::events::FOCUS),
        "{:?}",
        ctx.focus_request()
    );
    let (layout, _) = render(&mut app, 160);
    assert!(
        layout
            .focus_targets()
            .iter()
            .any(|target| target.id.as_str() == crate::app::events::FOCUS)
    );
}

#[test]
fn retained_acceptance_conversations_require_recreation_before_opening() {
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
        ..Default::default()
    };
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        acceptances: vec![acceptance.clone()],
        workspaces: [(acceptance.id, workspace.clone())].into(),
        ..Default::default()
    });
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
                ..Default::default()
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
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Enter.into()), &mut ctx);
    assert!(matches!(ctx.messages(), [Msg::OpenEventLink(row)] if row.sequence == 7));
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('d').into()), &mut ctx);
    assert!(matches!(ctx.messages(), [Msg::OpenEvent(row)] if row.sequence == 7));
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
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('d').into()), &mut ctx);
    assert!(matches!(ctx.messages(), [Msg::FocusRule(name)] if name == "inspect"));
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Enter.into()), &mut ctx);
    assert!(
        matches!(ctx.messages(), [Msg::AcceptanceAction(target, Command::Routes)] if target.acceptance.id == 71 && target.selected.is_none())
    );
    for (key, command) in [
        ('r', Command::Rule),
        ('v', Command::Instance),
        ('n', Command::NewSession),
        ('p', Command::PurgeInstance),
    ] {
        let mut ctx = EventCtx::default();
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char(key).into()), &mut ctx);
        assert!(
            matches!(ctx.messages(), [Msg::AcceptanceAction(target, action)] if target.acceptance.id == 71 && *action == command)
        );
    }
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
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('d').into()), &mut ctx);
    assert!(
        matches!(ctx.messages(), [Msg::AcceptanceAction(target, Command::Details)] if target.selected.as_ref().unwrap().label.contains("Active task"))
    );
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('o').into()), &mut ctx);
    assert!(ctx.messages().is_empty());
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('c').into()), &mut ctx);
    assert!(
        matches!(ctx.messages(), [Msg::AcceptanceAction(target, Command::ClosePanel)] if matches!(target.selected.as_ref().unwrap().opencode.as_ref(), Some(crate::app::opencode::Target::Session { pane: Some(pane), .. }) if pane.id == 2))
    );
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('.').into()), &mut ctx);
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    let (menu_layout, text) = render(&mut app, 160);
    assert!(
        text.lines()
            .any(|line| line.contains("Go to OpenCode session")
                && line.trim_end().ends_with("Enter")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.contains("Close OpenCode panel") && line.trim_end().ends_with('c')),
        "{text}"
    );
    app.opencode_action = Some(crate::app::opencode::PendingAction::new(
        tokio::sync::oneshot::channel().1,
        "Cannot close OpenCode session",
        Vec::new(),
    ));
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(menu_layout.overlays().last().unwrap().route_path.clone()),
        &TuiEvent::Key(Key::Char('c').into()),
        &mut ctx,
    );
    assert!(!app.menu_layer().layer().is_open());
    app.opencode_action = None;
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Enter.into()), &mut ctx);
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
fn saved_acceptance_conversations_render_compactly_with_cached_client_samples() {
    init_ui();
    let mut session = conversation("ses_original", "Original task");
    session.last_question = Some("Check the gateway".into());
    session.question_observed = true;
    session.panes = vec![Pane {
        session: "main".into(),
        id: 7,
        tab_id: 1,
        tab_name: "Inspect".into(),
    }];
    let mut context = Context {
        inventory: snapshot(),
        opencode: crate::store::opencode::Snapshot {
            sessions: vec![session],
            resources: vec![crate::store::opencode::resources::ProcessResource {
                pid: 42,
                session_id: "ses_original".into(),
                directory: "/tmp/workspaces/review".into(),
                zellij_session: "main".into(),
                pane_id: Some(7),
                usage: UsageSummary {
                    memory_bytes: Some(200 * 1048576),
                    cpu_basis_points: Some(200),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        },
    };
    let attached = Target::new(&acceptance(), None, &context, true);
    assert_eq!(attached.conversations[0].height(), 2);
    assert!(
        attached.conversations[0]
            .text("", None)
            .to_string()
            .contains("Check the gateway")
    );

    context.opencode.sessions[0].panes.clear();
    let mut saved = Target::new(&acceptance(), None, &context, true);
    let row = &saved.conversations[0];
    assert_eq!(row.height(), 1);
    assert_eq!(row.text("", None).to_string(), "󰚩 Original task");
    assert!(row.memory_text_with_spinner("").spans.is_empty());
    assert!(row.cpu_text_with_spinner("").spans.is_empty());
    assert!(
        Target::new(&acceptance(), None, &context, false)
            .conversations
            .is_empty()
    );
    saved.selected = Some(row.clone());
    assert!(!saved.enabled(Command::ClosePanel));
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
        let mut activation = EventCtx::default();
        app.dispatch_event(
            &route,
            &TuiEvent::Key(Key::Char('d').into()),
            &mut activation,
        );
        if history {
            assert!(matches!(activation.messages(), [Msg::FocusEvent(7)]));
        } else {
            assert!(matches!(activation.messages(), [Msg::FocusRule(name)] if name == "inspect"));
        }
        let mut menu = EventCtx::default();
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char('.').into()), &mut menu);
        for message in menu.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
        let (menu_layout, text) = render(&mut app, 160);
        assert!(
            text.lines().any(|line| {
                line.contains("Open routes")
                    && line.trim_end_matches([' ', '┃', '│']).ends_with("Enter")
            }),
            "{text}"
        );
        assert!(!text.contains("Go to OpenCode session"), "{text}");
        app.dispatch_event(
            &EventRoute::new(menu_layout.overlays().last().unwrap().route_path.clone()),
            &TuiEvent::Key(Key::Esc.into()),
            &mut EventCtx::default(),
        );
        app.dispatch_focus(focus, true, &mut tuicore::FocusCtx::default());
        let mut ctx = EventCtx::default();
        app.dispatch_event(&route, &TuiEvent::Key(Key::Enter.into()), &mut ctx);
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
            &TuiEvent::Key(Key::Char('d').into()),
            &mut ctx,
        );
        assert!(
            matches!(ctx.messages(), [Msg::OpenRule(rule)] if rule.definition.name == "inspect")
        );
    }
}

#[test]
fn historical_acceptances_show_report_outcomes_and_offer_report_reads_without_an_instance() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    let mut acceptance = acceptance();
    acceptance.status = crate::store::rules::DispatchStatus::Launching;
    let report = crate::store::rules::reports::ReportSummary {
        acceptance_id: acceptance.id,
        event_sequence: acceptance.event_sequence,
        rule_name: acceptance.rule_name.clone(),
        title: "Incident reviewed".into(),
        summary: "Verification evidence retained".into(),
        reported_at: "2026-10-04T00:00:00Z".into(),
        cleanup_state: crate::store::rules::reports::CleanupState::Failed,
        cleanup_error: Some("Client closure failed".into()),
    };
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        acceptances: vec![acceptance.clone()],
        reports: [(acceptance.id, report.clone())].into(),
        ..Default::default()
    });
    let target =
        Target::new(&acceptance, None, &Context::default(), true).with_report(Some(&report));
    assert!(target.enabled(Command::Report));
    assert!(target.enabled(Command::Delete));
    assert!(target.search().contains("Client closure failed"));
    let text = target.text("", Some(160)).to_string();
    assert!(
        text.contains("Launching") && text.contains("Reported · cleanup Failed"),
        "{text}"
    );
    assert!(
        text.contains("Incident reviewed") && text.contains("Verification evidence retained"),
        "{text}"
    );
    assert_eq!(target.height(), 2);
    let mut ctx = EventCtx::default();
    target.action(&TuiEvent::Key(Key::Char('f').into()), &mut ctx);
    assert!(matches!(
        ctx.messages(),
        [Msg::AcceptanceAction(_, Command::Report)]
    ));
    app.acceptance_action(target, Command::Report, &mut EventCtx::default());
    let (_, text) = render(&mut app, 160);
    assert!(
        text.contains("Acceptance report") && text.contains("Loading report"),
        "{text}"
    );
    assert!(
        [
            "Title",
            "Summary",
            "Report",
            "Incident reviewed",
            "Verification evidence retained",
            "Client closure failed"
        ]
        .iter()
        .all(|value| text.contains(value)),
        "{text}"
    );
    for width in [160, 56, 160] {
        let (_, text) = render(&mut app, width);
        let header = text
            .lines()
            .position(|line| line.contains("Acceptance report"))
            .unwrap();
        assert_eq!(header, 6, "{text}");
        let line = text.lines().nth(header).unwrap();
        let panel_width = width * crate::app::details_width_percent(width) / 100;
        assert_eq!(
            line.trim().chars().count(),
            usize::from(panel_width),
            "{text}"
        );
    }
    let mut pending = report;
    pending.cleanup_state = crate::store::rules::reports::CleanupState::Purging;
    assert!(
        !Target::new(&acceptance, None, &Context::default(), true)
            .with_report(Some(&pending))
            .enabled(Command::Delete)
    );
}
