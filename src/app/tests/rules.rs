use super::events::render;
use super::*;
use crate::store::{
    events::{Attempt, Batch, ProcessingStatus},
    rules::{Acceptance, Definition, DispatchStatus, Rule},
};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

pub(super) fn rule(name: &str) -> Rule {
    Rule {
        definition: Definition {
            name: name.into(),
            description: format!("{name} event handler"),
            script: "fn matches(event) { event.profile == \"message\" }".into(),
            template: "website".into(),
            model: "openai/test".into(),
            variant: None,
            initial_prompt: "Inspect {{event.data.text}}".into(),
            enabled: false,
            start_instance: true,
            focus_pane: true,
        },
        revision: 1,
        zellij_session: "main".into(),
    }
}

#[test]
fn bracket_navigation_reaches_rules_and_wraps_in_both_integration_modes() {
    init_ui();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for enabled in [true, false] {
        let service = AppService::for_tests();
        runtime
            .block_on(service.set_opencode_enabled(enabled).unwrap())
            .unwrap()
            .unwrap();
        let mut app = crate::app::root(service);
        for width in [40, 130] {
            for key in ['[', ']'] {
                app.tabs_mut().select_index(0);
                app.after_event(&mut EventCtx::default());
                let count = if enabled { 5 } else { 4 };
                for step in 1..=count {
                    let (layout, _) = render(&mut app, width);
                    let target = layout
                        .focus_targets()
                        .iter()
                        .find(|target| target.id.as_str() == "tabs")
                        .unwrap();
                    app.dispatch_event(
                        &EventRoute::new(target.path.clone()),
                        &TuiEvent::Key(Key::Char(key).into()),
                        &mut EventCtx::default(),
                    );
                    let expected = if key == ']' {
                        step % count
                    } else {
                        (count - step) % count
                    };
                    assert_eq!(app.tabs_mut().selected_index(), expected);
                    assert_eq!(app.rules_active, expected == if enabled { 3 } else { 2 });
                    if app.rules_active {
                        let (layout, _) = render(&mut app, width);
                        assert!(
                            layout
                                .focus_targets()
                                .iter()
                                .any(|target| target.id.as_str() == crate::app::rules::FOCUS)
                        );
                    }
                }
            }
        }
    }
}

pub(super) fn acceptance(rule: Rule, id: i64, sequence: i64) -> Acceptance {
    Acceptance {
        id,
        event_sequence: sequence,
        event_summary: format!("event-{sequence}"),
        attempt_id: 1,
        rule_name: rule.definition.name.clone(),
        rule_revision: 1,
        accepted_at: "2026-10-03T12:00:00Z".into(),
        instance: format!("event-{id}"),
        session_id: Some(format!("ses_{id}")),
        pane: None,
        operation_id: Some(format!("operation-{id}")),
        launch_started_at: Some("2026-10-03T12:00:00Z".into()),
        status: DispatchStatus::Launched,
        error: None,
        rule,
        resolved_prompt: Some("Inspect the event".into()),
    }
}

#[test]
fn rule_dialog_lists_only_its_acceptances_and_uses_bottom_docked_form_tabs() {
    let mut app = crate::app::root(AppService::for_tests());
    let first = rule("first");
    let second = rule("second");
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        rules: vec![first.clone(), second.clone()],
        acceptances: vec![acceptance(first.clone(), 1, 7), acceptance(second, 2, 7)],
        ..Default::default()
    });
    let index = app.rules_tab_index();
    app.tabs_mut().select_index(index);
    app.after_event(&mut EventCtx::default());
    render(&mut app, 130);
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let (layout, text) = render(&mut app, 130);
    assert!(
        text.contains("first event handler") && text.contains("second event handler"),
        "{text}"
    );
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::rules::FOCUS)
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(Key::Char('a').into()),
        &mut ctx,
    );
    assert!(ctx.messages().iter().any(|message| matches!(message, Msg::SaveRule(rule) if rule.definition.enabled && rule.definition.name == "first")));
    ctx.drain_messages().for_each(drop);
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(Key::Enter.into()),
        &mut ctx,
    );
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    for width in [130, 40, 80] {
        let (_, text) = render(&mut app, width);
        assert!(text.contains("Rule details"), "{text}");
        assert!(text.contains("first #1 · event #7"), "{text}");
        assert!(!text.contains("second #2"), "{text}");
        let border = text
            .lines()
            .position(|line| line.contains("Rule details"))
            .unwrap();
        assert!(border > 0, "{text}");
    }
    app.handle_message(Msg::Close, &mut EventCtx::default());
    app.open_rule(first, &mut EventCtx::default());
    let (_, text) = render(&mut app, 130);
    assert!(
        text.contains("Accepted events") && text.contains("Script") && text.contains("Settings"),
        "{text}"
    );
}

#[test]
fn acceptance_instance_navigation_preserves_filters_unless_the_assigned_instance_is_hidden() {
    init_ui();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for (enabled, running, active_only) in [true, false].into_iter().flat_map(|enabled| {
        [
            (enabled, true, true),
            (enabled, false, true),
            (enabled, true, false),
            (enabled, false, false),
        ]
    }) {
        let service = AppService::for_tests();
        runtime
            .block_on(service.set_opencode_enabled(enabled).unwrap())
            .unwrap()
            .unwrap();
        service.set_opencode_snapshot_for_tests(Default::default());
        let mut inventory = snapshot();
        let mut assigned = inventory.instances[0].clone();
        assigned.name = "assigned".into();
        assigned.workspace = "/tmp/workspaces/assigned".into();
        assigned.project = "tandem-assigned".into();
        if !running {
            for service in &mut assigned.services {
                service.status = "exited".into();
                service.health = None;
            }
        }
        inventory.instances.push(assigned);
        let mut app = crate::app::root(service);
        app.update_snapshot(inventory);
        app.set_running_only(active_only, &mut EventCtx::default());
        let selected = rule("selected");
        let mut row = acceptance(selected.clone(), 107, 42);
        row.instance = "assigned".into();
        app.pages_mut().update_rules(crate::store::rules::Snapshot {
            rules: vec![selected.clone()],
            acceptances: vec![row],
            ..Default::default()
        });
        app.open_rule(selected, &mut EventCtx::default());
        let (layout, _) = render(&mut app, 130);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == "acceptance-list")
            .unwrap();
        app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
        let mut ctx = EventCtx::default();
        app.dispatch_event(
            &EventRoute::new(target.path.clone()),
            &TuiEvent::Key(Key::Char('v').into()),
            &mut ctx,
        );
        for message in ctx.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
        let (_, text) = render(&mut app, 130);
        assert_eq!(app.tabs_mut().selected_index(), if enabled { 2 } else { 0 });
        assert!(!app.view.is_active());
        assert_eq!(app.running_only, active_only && running);
        assert_eq!(
            app.toolbar_state.borrow().running_only,
            active_only && running
        );
        assert_eq!(app.selected().unwrap().id, "instance:assigned");
        assert!(text.contains("assigned"), "{text}");
    }
}

#[test]
fn rule_history_menus_offer_instance_navigation_and_routes() {
    use crate::app::row_actions::Command;

    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    app.update_snapshot(snapshot());
    app.opencode_snapshot = crate::store::opencode::Snapshot {
        sessions: vec![crate::store::opencode::Session {
            id: "ses_selected".into(),
            title: "Inspect event".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: crate::store::opencode::Activity::Idle,
            ..Default::default()
        }],
        ..Default::default()
    };
    app.update_overview_rows(&snapshot(), &[], false);
    let selected = rule("selected");
    let mut row = acceptance(selected.clone(), 107, 42);
    row.instance = "review".into();
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        rules: vec![selected.clone()],
        acceptances: vec![row.clone()],
        ..Default::default()
    });
    app.open_rule(selected, &mut EventCtx::default());
    let (layout, _) = render(&mut app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == "acceptance-list")
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let route = EventRoute::new(target.path.clone());
    let mut ctx = EventCtx::default();
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('v').into()), &mut ctx);
    assert!(
        matches!(ctx.messages(), [Msg::AcceptanceAction(target, Command::Instance)] if target.acceptance == row)
    );
    for command in [Command::Instance, Command::Routes] {
        let mut ctx = EventCtx::default();
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char('.').into()), &mut ctx);
        assert!(ctx.messages().is_empty());
        let (menu_layout, text) = render(&mut app, 130);
        for (label, key) in [
            ("Go to rule", "r"),
            ("Open routes", "Enter"),
            ("Go to instance", "v"),
        ] {
            let line = text
                .lines()
                .find(|line| line.contains(&format!("{label}   ")))
                .unwrap_or_else(|| panic!("Missing {label}:\n{text}"));
            assert!(
                line.trim_end_matches([' ', '┃', '│']).ends_with(key),
                "{text}"
            );
        }
        let menu = menu_layout
            .focus_targets()
            .iter()
            .find(|target| target.enabled && target.id.as_str() == "input")
            .unwrap();
        app.dispatch_focus(menu, true, &mut tuicore::FocusCtx::default());
        let menu_route = EventRoute::new(menu.path.clone());
        let mut selection = EventCtx::default();
        for character in command.label().chars() {
            app.dispatch_event(
                &menu_route,
                &TuiEvent::Key(Key::Char(character).into()),
                &mut selection,
            );
        }
        app.dispatch_event(
            &menu_route,
            &TuiEvent::Key(Key::Enter.into()),
            &mut selection,
        );
        match command {
            Command::Instance => assert!(
                matches!(selection.messages(), [Msg::AcceptanceAction(target, Command::Instance)] if target.acceptance == row)
            ),
            Command::Routes => {
                assert!(
                    matches!(selection.messages(), [Msg::AcceptanceAction(target, Command::Routes)] if target.acceptance == row)
                )
            }
            _ => unreachable!(),
        }
        assert!(app.view.is_active());
        assert!(!render(&mut app, 130).1.contains("Go to instance"));
    }
    app.dispatch_event(
        &route,
        &TuiEvent::Key(Key::Char('.').into()),
        &mut EventCtx::default(),
    );
    let (menu_layout, _) = render(&mut app, 130);
    let menu = menu_layout
        .focus_targets()
        .iter()
        .find(|target| target.enabled && target.id.as_str() == "input")
        .unwrap();
    app.dispatch_focus(menu, true, &mut tuicore::FocusCtx::default());
    let mut cancel = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(menu.path.clone()),
        &TuiEvent::Key(Key::Esc.into()),
        &mut cancel,
    );
    assert!(cancel.messages().is_empty());
    assert!(app.view.is_active());
    let (layout, text) = render(&mut app, 130);
    assert!(!text.contains("Go to instance"));
    assert!(
        layout
            .focus_targets()
            .iter()
            .any(|target| { target.enabled && target.id.as_str() == "acceptance-list" })
    );
}

#[test]
fn acceptance_menus_require_observed_targets_and_verified_inventory() {
    use crate::app::row_actions::{Command, Target};

    init_ui();
    let row = acceptance(rule("selected"), 107, 42);
    let mut context = crate::app::acceptances::Context::default();
    let target = Target::AcceptanceContext(Box::new(crate::app::acceptances::Target::new(
        &row, None, &context, true,
    )));
    assert!(!target.enabled(Command::CreateInstance));
    context.inventory.observed_at_unix_seconds = Some(1);
    let target = Target::AcceptanceContext(Box::new(crate::app::acceptances::Target::new(
        &row, None, &context, true,
    )));
    assert!(target.enabled(Command::CreateInstance));
    assert!(!target.enabled(Command::Instance));
    assert!(!target.enabled(Command::NewSession));
    assert!(!target.enabled(Command::PurgeInstance));
    assert!(!target.enabled(Command::Session));
    context.inventory.loading = true;
    let target = Target::AcceptanceContext(Box::new(crate::app::acceptances::Target::new(
        &row, None, &context, true,
    )));
    assert!(!target.enabled(Command::CreateInstance));
}

#[test]
fn prompt_editor_highlights_handlebars_paths_and_blocks() {
    init_ui();
    let mut app = crate::app::root(AppService::for_tests());
    let mut rule = rule("highlighted");
    rule.definition.initial_prompt =
        "Inspect {{event.data.text}}\n{{#if event.data.thread}}Thread{{/if}}".into();
    app.open_rule(rule, &mut EventCtx::default());
    show_settings(&mut app);
    let area = Rect::new(0, 0, 130, 60);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut layout = tuicore::LayoutCtx::new();
        layout.with_overlay_bounds(area, |ctx| app.layout(area, ctx));
        app.tick(Duration::from_millis(10), AnimationSettings::default());
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                app.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        let y = lines
            .iter()
            .position(|line| line.contains("Inspect {{event.data.text}}"))
            .unwrap();
        let x = lines[y][..lines[y].find("Inspect").unwrap()]
            .chars()
            .count();
        let buffer = terminal.backend().buffer();
        let prose = buffer[(x as u16, y as u16)].fg;
        let path_colored =
            (x + 7..x + 26).any(|column| buffer[(column as u16, y as u16)].fg != prose);
        let block_colored =
            (x..x + 26).any(|column| buffer[(column as u16, y as u16 + 1)].fg != prose);
        if path_colored && block_colored {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Handlebars highlighting did not appear"
        );
        std::thread::yield_now();
    }
}

#[test]
fn rule_backgrounds_alternate_across_both_lines_and_follow_search_results() {
    init_ui();
    let shared = Rc::new(RefCell::new(crate::store::rules::Snapshot {
        rules: ["keep-first", "drop-second", "keep-third", "keep-fourth"]
            .into_iter()
            .map(rule)
            .collect(),
        ..Default::default()
    }));
    let mut view =
        crate::app::rules::Rules::new(shared, AppService::for_tests().environment_keys());
    let area = Rect::new(0, 0, 100, 20);
    for search in [false, true] {
        if search {
            view.focus(None, true, &mut tuicore::FocusCtx::default());
            for character in "/keep".chars() {
                view.event(
                    &TuiEvent::Key(Key::Char(character).into()),
                    &mut EventCtx::default(),
                );
            }
            view.focus(None, false, &mut tuicore::FocusCtx::default());
        }
        view.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                view.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        let names = if search {
            ["keep-third", "keep-fourth"]
        } else {
            ["drop-second", "keep-third"]
        };
        for (index, name) in names.into_iter().enumerate() {
            let y = lines.iter().position(|line| line.contains(name)).unwrap() as u16;
            let background = terminal.backend().buffer()[(80, y)].bg;
            assert_eq!(background, terminal.backend().buffer()[(80, y + 1)].bg);
            if index == 0 {
                assert_ne!(background, tuicore::theme().surface_bg());
            } else {
                assert_eq!(background, tuicore::theme().surface_bg());
            }
        }
    }
}

#[test]
fn acceptance_navigation_loads_a_retained_event_and_clears_conflicting_source_filters() {
    let service = AppService::for_tests();
    let token = service.register_provider_for_tests("alpha");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut sequence = 0;
    for batch in 0..3 {
        let events = (0..100)
            .map(|index| {
                crate::environments::events::tests::event(&format!("event-{}", batch * 100 + index))
            })
            .collect();
        let response = runtime
            .block_on(service.ingest_events(token.clone(), Batch { events }))
            .unwrap();
        if batch == 0 {
            sequence = response.receipts[0].sequence;
        }
    }
    let feed = runtime.block_on(service.list_events()).unwrap();
    assert_eq!(feed.records.len(), 200);
    assert!(!feed.records.iter().any(|row| row.sequence == sequence));
    let mut app = crate::app::root(service);
    app.pages_mut().update_events(feed);
    app.handle_message(
        Msg::ProviderStreamEvents("beta".into(), "samples".into()),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    app.handle_message(Msg::FocusEvent(sequence), &mut EventCtx::default());
    let deadline = Instant::now() + Duration::from_secs(2);
    while !app.poll_event_focus() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let (layout, text) = render(&mut app, 130);
    assert_eq!(app.tabs_mut().selected_index(), 1);
    assert!(
        text.contains("alpha") && text.contains("201 of 300 events"),
        "{text}"
    );
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(Key::Char('d').into()),
        &mut ctx,
    );
    assert!(
        ctx.messages()
            .iter()
            .any(|message| matches!(message, Msg::OpenEvent(row) if row.sequence == sequence)),
        "{:?}",
        ctx.messages()
    );
}

#[test]
fn event_rows_summarize_acceptances_and_tree_children_navigate_to_their_rule() {
    let mut app = crate::app::root(AppService::for_tests());
    let acceptances: Vec<_> = (0..14)
        .map(|index| {
            let name = if index < 4 || index % 2 == 1 {
                "third"
            } else {
                "first"
            };
            let mut row = acceptance(rule(name), index + 1, 7);
            row.attempt_id = index + 1;
            if index >= 4 {
                row.status = DispatchStatus::Uncertain;
            }
            row
        })
        .collect();
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        rules: ["first", "second", "third"].into_iter().map(rule).collect(),
        ..Default::default()
    });
    let event = crate::store::events::Record {
        sequence: 7,
        provider: "alpha".into(),
        received_at: "now".into(),
        event: crate::environments::events::tests::event("sample"),
        attempts: vec![Attempt {
            id: 14,
            status: ProcessingStatus::Accepted,
            replay: false,
            created_at: "now".into(),
            accepted_at: Some("now".into()),
        }],
        acceptances,
    };
    app.pages_mut()
        .update_events(crate::store::events::Snapshot {
            records: vec![event.clone()],
            total: 1,
            ..Default::default()
        });
    app.handle_message(
        Msg::ProviderStreamEvents("alpha".into(), "samples".into()),
        &mut EventCtx::default(),
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let (_, text) = render(&mut app, 130);
    assert!(
        text.contains("2 rules · 4 launches · 10 uncertain"),
        "{text}"
    );
    let (layout, _) = render(&mut app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::events::FOCUS)
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::default();
    let route = EventRoute::new(target.path.clone());
    for key in [Key::Right, Key::Down] {
        app.dispatch_event(&route, &TuiEvent::Key(key.into()), &mut ctx);
    }
    app.dispatch_event(&route, &TuiEvent::Key(Key::Char('d').into()), &mut ctx);
    assert!(matches!(ctx.messages(), [Msg::FocusRule(name)] if name == "third"));
    let mut navigation = EventCtx::default();
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut navigation);
    }
    assert_eq!(app.tabs_mut().selected_index(), app.rules_tab_index());
    assert!(!app.view.is_active());
    assert_eq!(
        navigation.focus_request(),
        Some(&tuicore::FocusRequest::Target(tuicore::FocusId::new(
            crate::app::rules::FOCUS
        )))
    );
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
    let (layout, _) = render(&mut app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::rules::FOCUS)
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(Key::Char('d').into()),
        &mut ctx,
    );
    assert!(matches!(ctx.messages(), [Msg::OpenRule(rule)] if rule.definition.name == "third"));
}

#[test]
fn event_rows_show_each_dispatch_outcome_with_singular_rule_and_launch_labels() {
    init_ui();
    let statuses = [
        DispatchStatus::Launched,
        DispatchStatus::Uncertain,
        DispatchStatus::Queued,
        DispatchStatus::Provisioning,
        DispatchStatus::Launching,
        DispatchStatus::Failed,
    ];
    let event = crate::store::events::Record {
        sequence: 7,
        provider: "alpha".into(),
        received_at: "now".into(),
        event: crate::environments::events::tests::event("sample"),
        attempts: vec![],
        acceptances: statuses
            .into_iter()
            .enumerate()
            .map(|(index, status)| {
                let mut row = acceptance(rule("first"), index as i64 + 1, 7);
                row.status = status;
                row
            })
            .collect(),
    };
    let text = crate::app::events::row_text(&event, None).to_string();
    assert!(
        text.contains(
            "1 rule · 1 launch · 1 uncertain · 1 queued · 1 provisioning · 1 launching · 1 failed"
        ),
        "{text}"
    );
}

fn editor(enabled: bool) -> App {
    let mut service = AppService::for_tests();
    service.set_rule_session_for_tests("active-rule-session");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(service.create_template("website".into()))
        .unwrap();
    let mut definition = rule("editable").definition;
    definition.enabled = enabled;
    let rule = service
        .save_rule(definition, None, Some("main".into()), enabled)
        .blocking_recv()
        .unwrap()
        .unwrap();
    let mut app = crate::app::root(service);
    app.open_rule(rule, &mut EventCtx::default());
    app
}

fn show_rules(app: &mut App, rules: Vec<Rule>) {
    app.pages_mut().update_rules(crate::store::rules::Snapshot {
        rules,
        ..Default::default()
    });
    let index = app.rules_tab_index();
    app.tabs_mut().select_index(index);
    app.after_event(&mut EventCtx::default());
    app.pages_mut()
        .tick(Duration::ZERO, AnimationSettings::default());
}

#[test]
fn rule_menu_and_row_hotkey_offer_the_action_for_the_selected_state() {
    use crate::app::row_actions::{Command, Target};
    for enabled in [false, true] {
        let mut selected = rule("selected");
        selected.definition.enabled = enabled;
        let target = Target::Rule(Box::new(selected.clone()));
        let command = if enabled {
            Command::Deactivate
        } else {
            Command::Activate
        };
        assert!(target.commands() == [Command::Details, command]);
        assert!(target.enabled(command));
        assert!(!target.enabled(if enabled {
            Command::Activate
        } else {
            Command::Deactivate
        }));
        let mut app = editor(enabled);
        app.handle_message(Msg::Close, &mut EventCtx::default());
        let saved = saved_rule(&app);
        show_rules(&mut app, vec![saved]);
        let (layout, _) = render(&mut app, 130);
        let target = layout
            .focus_targets()
            .iter()
            .find(|target| target.id.as_str() == crate::app::rules::FOCUS)
            .unwrap();
        app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
        let route = EventRoute::new(target.path.clone());
        let mut ctx = EventCtx::default();
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char('a').into()), &mut ctx);
        assert!(
            matches!(ctx.messages(), [Msg::SaveRule(rule)] if rule.definition.enabled != enabled)
        );
        ctx.drain_messages().for_each(drop);
        app.dispatch_event(&route, &TuiEvent::Key(Key::Char('.').into()), &mut ctx);
        for message in ctx.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
        let text = render(&mut app, 130).1;
        for (label, key) in [("View details", "d"), (command.label(), "a")] {
            let line = text
                .lines()
                .find(|line| line.contains(&format!("{label}   ")))
                .unwrap();
            assert!(line.trim_end_matches([' ', '┃']).ends_with(key), "{text}");
            if label == "View details" {
                assert!(line.contains("Enter / d"), "{line}");
            }
        }
        for character in command.label().chars() {
            app.event(
                &TuiEvent::Key(Key::Char(character).into()),
                &mut EventCtx::default(),
            );
        }
        let mut ctx = EventCtx::default();
        app.event(&TuiEvent::Key(Key::Enter.into()), &mut ctx);
        assert!(!app.menu_layer().is_active());
        assert!(!app.view.is_active());
        assert!(app.rule_save.is_some());
        let (layout, _) = render(&mut app, 130);
        let mut focus = tuicore::FocusManager::new();
        let mut dispatcher = tuicore::TreeDispatcher::new();
        let transition = focus
            .apply_request(
                &tuicore::FocusRequest::Target(tuicore::FocusId::new(crate::app::rules::FOCUS)),
                layout.focus_targets(),
            )
            .unwrap();
        dispatcher.dispatch_focus(&mut app, transition, AnimationSettings::default());
        finish_autosaves(&mut app);
        assert_eq!(saved_rule(&app).definition.enabled, !enabled);
        let notice = app.notifications.center().history().last().unwrap();
        assert_eq!(
            notice.title(),
            if enabled {
                "Rule deactivated"
            } else {
                "Rule activated"
            }
        );
        assert_eq!(notice.body(), "editable");
        assert_eq!(notice.kind(), tuicore::NotificationKind::Success);
        assert_eq!(
            saved_rule(&app).zellij_session,
            if enabled {
                "main"
            } else {
                "active-rule-session"
            }
        );
        for _ in 0..4 {
            app.tick(Duration::from_millis(16), AnimationSettings::default());
            let (_, text) = render(&mut app, 130);
            let row = text
                .lines()
                .find(|line| line.contains("editable 󰠲"))
                .unwrap();
            let icon = if enabled { "" } else { "" };
            assert!(
                row.contains(&format!(
                    "{icon} editable 󰠲 website 󰧑 openai/test · default · 󰒋"
                )),
                "{text}"
            );
            assert_eq!(
                focus.current().unwrap().id.as_str(),
                crate::app::rules::FOCUS
            );
            assert!(!app.view.is_active());
        }
        let effects = dispatcher.dispatch_event(
            &mut app,
            &EventRoute::new(focus.current_path()),
            &TuiEvent::Key(Key::Char('d').into()),
            AnimationSettings::default(),
        );
        assert!(
            matches!(effects.messages.as_slice(), [Msg::OpenRule(rule)] if rule.definition.name == "editable")
        );
    }
}

#[test]
fn rule_bulk_control_uses_one_slot_and_hotkey_for_aggregate_state_and_ignores_search() {
    let mut app = crate::app::root(AppService::for_tests());
    for width in [40, 130] {
        let mut slot = None;
        for enabled in [[false, false], [true, false], [true, true]] {
            let rules = ["first", "second"]
                .into_iter()
                .zip(enabled)
                .map(|(name, enabled)| {
                    let mut rule = rule(name);
                    rule.definition.enabled = enabled;
                    rule
                })
                .collect();
            show_rules(&mut app, rules);
            let (layout, text) = render(&mut app, width);
            let activate = !enabled.contains(&true);
            let label = if activate {
                "Activate all"
            } else {
                "Deactivate all"
            };
            let line = text.lines().find(|line| line.contains(label)).unwrap();
            assert!(line.contains("|A|"), "{text}");
            let button = layout
                .focus_targets()
                .iter()
                .find(|target| {
                    target
                        .path
                        .keys()
                        .iter()
                        .any(|key| key.as_str() == "rule-activate-all")
                })
                .unwrap();
            assert!(button.enabled && button.tab_stop);
            if let Some(slot) = &slot {
                assert_eq!(&button.path, slot);
            }
            slot = Some(button.path.clone());
            let route = EventRoute::new(button.path.clone());
            let mut ctx = EventCtx::default();
            app.dispatch_focus(button, true, &mut tuicore::FocusCtx::default());
            app.dispatch_event(&route, &TuiEvent::Key(Key::Enter.into()), &mut ctx);
            assert!(matches!(ctx.messages(), [Msg::RuleBulkAction(value)] if *value == activate));
            ctx.drain_messages().for_each(drop);
            app.dispatch_event(&route, &TuiEvent::Key(Key::Esc.into()), &mut ctx);
            assert_eq!(
                ctx.focus_request(),
                Some(&tuicore::FocusRequest::Target(tuicore::FocusId::new(
                    crate::app::rules::FOCUS
                )))
            );
            let target = layout
                .focus_targets()
                .iter()
                .find(|target| target.id.as_str() == crate::app::rules::FOCUS)
                .unwrap();
            app.dispatch_focus(button, false, &mut tuicore::FocusCtx::default());
            app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
            let route = EventRoute::new(target.path.clone());
            let mut ctx = EventCtx::default();
            app.dispatch_event(&route, &TuiEvent::Key(Key::Char('A').into()), &mut ctx);
            assert!(matches!(ctx.messages(), [Msg::RuleBulkAction(value)] if *value == activate));
            app.dispatch_event(
                &route,
                &TuiEvent::Key(Key::Char('/').into()),
                &mut EventCtx::default(),
            );
            let mut ctx = EventCtx::default();
            for key in [Key::Char('a'), Key::Char('A'), Key::Char('.')] {
                app.dispatch_event(&route, &TuiEvent::Key(key.into()), &mut ctx);
            }
            assert!(!ctx.messages().iter().any(|message| matches!(
                message,
                Msg::RuleBulkAction(_) | Msg::SaveRule(_) | Msg::OpenRowMenu(_)
            )));
            app.dispatch_event(
                &route,
                &TuiEvent::Key(Key::Esc.into()),
                &mut EventCtx::default(),
            );
        }
    }
    show_rules(&mut app, Vec::new());
    let (layout, _) = render(&mut app, 130);
    assert!(
        !layout
            .focus_targets()
            .iter()
            .find(|target| target
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "rule-activate-all"))
            .unwrap()
            .enabled
    );
}

#[test]
fn bulk_rule_authorization_pins_eligible_revisions_and_keeps_successful_siblings() {
    let mut app = editor(true);
    app.handle_message(Msg::Close, &mut EventCtx::default());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for name in ["first", "second", "third"] {
        app.service
            .save_rule(rule(name).definition, None, Some("main".into()), false)
            .blocking_recv()
            .unwrap()
            .unwrap();
    }
    let snapshot = runtime.block_on(app.service.list_rules()).unwrap();
    show_rules(&mut app, snapshot.rules.clone());
    app.handle_message(Msg::RuleBulkAction(true), &mut EventCtx::default());
    assert!(app.rule_save.is_none());
    assert_eq!(
        runtime.block_on(app.service.list_rules()).unwrap().rules,
        snapshot.rules
    );
    assert!(render(&mut app, 130).1.contains("Activate 3 rules"));
    app.handle_message(Msg::Close, &mut EventCtx::default());
    assert!(app.rule_saves.is_empty());
    app.handle_message(Msg::RuleBulkAction(true), &mut EventCtx::default());
    render(&mut app, 130);
    let mut ctx = EventCtx::default();
    app.event(&TuiEvent::Key(Key::Char('o').into()), &mut ctx);
    let targets = ctx
        .drain_messages()
        .find_map(|message| match message {
            Msg::SaveRules(rules) => Some(rules),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|rule| rule.definition.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "third"]
    );
    let second = snapshot
        .rules
        .iter()
        .find(|rule| rule.definition.name == "second")
        .unwrap();
    app.service
        .save_rule(
            second.definition.clone(),
            Some(second.revision),
            Some("main".into()),
            false,
        )
        .blocking_recv()
        .unwrap()
        .unwrap();
    app.handle_message(Msg::SaveRules(targets), &mut EventCtx::default());
    finish_autosaves(&mut app);
    let rules = runtime.block_on(app.service.list_rules()).unwrap().rules;
    assert_eq!(
        rules
            .iter()
            .map(|rule| (
                rule.definition.name.as_str(),
                rule.definition.enabled,
                rule.revision
            ))
            .collect::<Vec<_>>(),
        [
            ("editable", true, 1),
            ("first", true, 2),
            ("second", false, 2),
            ("third", true, 2)
        ]
    );
    for name in ["first", "third"] {
        assert_eq!(
            rules
                .iter()
                .find(|rule| rule.definition.name == name)
                .unwrap()
                .zellij_session,
            "active-rule-session"
        );
    }
    show_rules(&mut app, rules);
    app.handle_message(Msg::RuleBulkAction(false), &mut EventCtx::default());
    render(&mut app, 130);
    let mut ctx = EventCtx::default();
    app.event(&TuiEvent::Key(Key::Char('o').into()), &mut ctx);
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    finish_autosaves(&mut app);
    assert!(
        runtime
            .block_on(app.service.list_rules())
            .unwrap()
            .rules
            .iter()
            .all(|rule| !rule.definition.enabled)
    );
}

fn finish_autosaves(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.rule_save.is_some() {
        assert!(Instant::now() < deadline);
        app.poll_rule_save();
        std::thread::yield_now();
    }
}

fn saved_rule(app: &App) -> Rule {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(app.service.list_rules())
        .unwrap()
        .rules
        .remove(0)
}

fn show_settings(app: &mut App) {
    for _ in 0..2 {
        let (layout, _) = render(app, 130);
        let tabs = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target.id.as_str() == "tabs"
                    && target
                        .path
                        .keys()
                        .iter()
                        .any(|key| key.as_str() == "rule-tabs")
            })
            .unwrap();
        app.dispatch_event(
            &EventRoute::new(tabs.path.clone()),
            &TuiEvent::Key(Key::Char(']').into()),
            &mut EventCtx::default(),
        );
    }
}

fn settings_input(app: &mut App, slot: &str, keys: &[Key]) {
    let (layout, _) = render(app, 130);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.path.keys().iter().any(|key| key.as_str() == slot))
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    for key in keys {
        let mut ctx = EventCtx::default();
        app.dispatch_event(
            &EventRoute::new(target.path.clone()),
            &TuiEvent::Key((*key).into()),
            &mut ctx,
        );
        for message in ctx.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
    }
    app.dispatch_focus(target, false, &mut tuicore::FocusCtx::default());
}

#[test]
fn prompt_template_errors_appear_below_the_prompt_and_clear_when_corrected() {
    for enabled in [false, true] {
        let mut app = editor(enabled);
        show_settings(&mut app);
        let original = saved_rule(&app);
        let notices = app.notifications.center().history().len();
        settings_input(
            &mut app,
            "prompt",
            &[Key::Enter, Key::End, Key::Char('{'), Key::Char('{')],
        );
        finish_autosaves(&mut app);
        app.tick(Duration::ZERO, AnimationSettings::default());
        let (layout, text) = render(&mut app, 130);
        let prompt = layout
            .focus_targets()
            .iter()
            .find(|target| {
                target
                    .path
                    .keys()
                    .iter()
                    .any(|key| key.as_str() == "prompt")
            })
            .unwrap();
        let warning = text
            .lines()
            .position(|line| line.contains("Template error:"))
            .unwrap_or_else(|| {
                panic!(
                    "prompt error: {:?}\n{text}",
                    app.rule_editor.as_ref().unwrap().borrow().prompt_error
                )
            });
        assert!(warning as u16 >= prompt.area.bottom(), "{text}");
        assert_eq!(app.notifications.center().history().len(), notices);
        assert!(
            app.service
                .validate_rule_prompt(&saved_rule(&app).definition.initial_prompt)
                .is_ok()
        );
        assert!(!saved_rule(&app).definition.enabled);

        settings_input(
            &mut app,
            "prompt",
            &[Key::Enter, Key::Backspace, Key::Backspace],
        );
        finish_autosaves(&mut app);
        app.tick(Duration::ZERO, AnimationSettings::default());
        let text = render(&mut app, 130).1;
        assert!(!text.contains("Template error:"), "{text}");
        assert_eq!(app.notifications.center().history().len(), notices);
        assert_eq!(
            saved_rule(&app).definition.initial_prompt,
            original.definition.initial_prompt
        );
        assert_eq!(
            app.rule_editor.as_ref().unwrap().borrow().rule,
            saved_rule(&app)
        );
    }
}

fn assert_setting(app: &mut App, label: &str, checked: bool) {
    let text = render(app, 130).1;
    let line = text.lines().find(|line| line.contains(label)).unwrap();
    assert!(line.contains(if checked { "──●" } else { "○──" }), "{text}");
}

fn delay_save(
    app: &mut App,
) -> (
    tokio::sync::oneshot::Sender<Result<Rule, String>>,
    Result<Rule, String>,
) {
    let result = app.rule_save.take().unwrap().blocking_recv().unwrap();
    let (send, receive) = tokio::sync::oneshot::channel();
    app.rule_save = Some(receive);
    (send, result)
}

#[test]
fn focus_pane_toggle_preserves_activation_and_destination_and_rolls_back_failed_saves() {
    for enabled in [false, true] {
        for succeeds in [false, true] {
            let mut app = editor(enabled);
            show_settings(&mut app);
            let draft = app.rule_editor.as_ref().unwrap().clone();
            let original = draft.borrow().saved.clone();
            assert_setting(&mut app, "Focus new pane", true);
            if !succeeds {
                app.service
                    .save_rule(
                        original.definition.clone(),
                        Some(original.revision),
                        Some(original.zellij_session.clone()),
                        true,
                    )
                    .blocking_recv()
                    .unwrap()
                    .unwrap();
            }
            settings_input(&mut app, "focus-pane", &[Key::Char(' ')]);
            let (send, result) = delay_save(&mut app);
            assert_eq!(result.is_ok(), succeeds);
            app.tick(Duration::ZERO, AnimationSettings::default());
            assert_setting(&mut app, "Focus new pane", false);
            assert_setting(&mut app, "Active", enabled);
            if let Ok(saved) = &result {
                assert!(!saved.definition.focus_pane);
                assert_eq!(saved.definition.enabled, enabled);
                assert_eq!(saved.zellij_session, original.zellij_session);
            }
            send.send(result).unwrap();
            assert!(app.poll_rule_save());
            app.tick(Duration::ZERO, AnimationSettings::default());
            assert_setting(&mut app, "Focus new pane", !succeeds);
            assert_eq!(draft.borrow().rule.definition.enabled, enabled);
            assert!(draft.borrow().pending_toggle.is_none());
            if succeeds {
                settings_input(&mut app, "focus-pane", &[Key::Char(' ')]);
                finish_autosaves(&mut app);
                assert!(saved_rule(&app).definition.focus_pane);
            } else {
                assert_eq!(draft.borrow().rule, original);
            }
        }
    }
}

#[test]
fn start_instance_toggle_preserves_active_state_and_pinned_destination() {
    for enabled in [false, true] {
        let mut app = editor(enabled);
        show_settings(&mut app);
        let draft = app.rule_editor.as_ref().unwrap().clone();
        for start in [false, true] {
            let original = draft.borrow().saved.clone();
            settings_input(&mut app, "start-instance", &[Key::Char(' ')]);
            assert_eq!(draft.borrow().rule, original);
            let (send, result) = delay_save(&mut app);
            for _ in 0..3 {
                app.tick(Duration::ZERO, AnimationSettings::default());
                assert_setting(&mut app, "Start instance", start);
                assert_setting(&mut app, "Active", enabled);
            }
            let saved = result.as_ref().unwrap();
            assert_eq!(saved.definition.enabled, enabled);
            assert_eq!(saved.definition.start_instance, start);
            assert_eq!(saved.zellij_session, original.zellij_session);
            if start {
                app.handle_message(Msg::Close, &mut EventCtx::default());
                assert!(app.rule_editor.is_none());
            }
            send.send(result).unwrap();
            assert!(app.poll_rule_save());
            app.tick(Duration::ZERO, AnimationSettings::default());
            if !start {
                assert_setting(&mut app, "Start instance", start);
            }
            assert_eq!(draft.borrow().rule, saved_rule(&app));
            assert!(draft.borrow().pending_toggle.is_none());
        }
    }
}

#[test]
fn active_toggle_displays_requested_state_until_success_or_error_in_both_directions() {
    for enabled in [false, true] {
        for succeeds in [false, true] {
            let mut app = editor(enabled);
            show_settings(&mut app);
            let draft = app.rule_editor.as_ref().unwrap().clone();
            let original = draft.borrow().saved.clone();
            if !succeeds {
                app.service
                    .save_rule(
                        original.definition.clone(),
                        Some(original.revision),
                        Some(original.zellij_session.clone()),
                        true,
                    )
                    .blocking_recv()
                    .unwrap()
                    .unwrap();
            }
            settings_input(&mut app, "enabled", &[Key::Char(' ')]);
            assert_eq!(draft.borrow().rule, original);
            let (send, result) = delay_save(&mut app);
            assert_eq!(result.is_ok(), succeeds);
            for _ in 0..3 {
                app.tick(Duration::ZERO, AnimationSettings::default());
                assert_setting(&mut app, "Active", !enabled);
            }
            settings_input(&mut app, "start-instance", &[Key::Char(' ')]);
            settings_input(&mut app, "enabled", &[Key::Char(' ')]);
            app.tick(Duration::ZERO, AnimationSettings::default());
            assert_setting(&mut app, "Start instance", true);
            assert_setting(&mut app, "Active", !enabled);
            assert_eq!(draft.borrow().rule, original);
            send.send(result).unwrap();
            assert!(app.poll_rule_save());
            assert!(draft.borrow().pending_toggle.is_none());
            app.tick(Duration::ZERO, AnimationSettings::default());
            assert_setting(
                &mut app,
                "Active",
                if succeeds { !enabled } else { enabled },
            );
            if succeeds {
                assert_eq!(draft.borrow().rule, saved_rule(&app));
                assert_eq!(
                    draft.borrow().rule.zellij_session,
                    if enabled {
                        "main"
                    } else {
                        "active-rule-session"
                    }
                );
            } else {
                assert_eq!(draft.borrow().rule, original);
            }
        }
    }
}

#[test]
fn start_instance_save_merges_text_edits_and_completes_after_dialog_close() {
    for close in [false, true] {
        let mut app = editor(true);
        show_settings(&mut app);
        let draft = app.rule_editor.as_ref().unwrap().clone();
        settings_input(&mut app, "start-instance", &[Key::Char(' ')]);
        let (send, result) = delay_save(&mut app);
        settings_input(
            &mut app,
            "description",
            &[Key::Enter, Key::End, Key::Char('!')],
        );
        if close {
            app.handle_message(Msg::Close, &mut EventCtx::default());
        }
        send.send(result).unwrap();
        assert!(app.poll_rule_save());
        finish_autosaves(&mut app);
        let saved = saved_rule(&app);
        assert_eq!(saved.definition.description, "editable event handler!");
        assert!(!saved.definition.start_instance);
        assert!(!saved.definition.enabled);
        assert_eq!(saved.zellij_session, "main");
        assert_eq!(draft.borrow().saved, saved);
        assert_eq!(draft.borrow().rule, saved);
    }
}

#[test]
fn failed_start_instance_save_rolls_back_toggle_and_retains_text_edits() {
    let mut app = editor(true);
    show_settings(&mut app);
    let draft = app.rule_editor.as_ref().unwrap().clone();
    let original = saved_rule(&app);
    app.service
        .save_rule(
            original.definition.clone(),
            Some(original.revision),
            Some(original.zellij_session.clone()),
            true,
        )
        .blocking_recv()
        .unwrap()
        .unwrap();
    settings_input(&mut app, "start-instance", &[Key::Char(' ')]);
    let (send, result) = delay_save(&mut app);
    assert!(result.is_err());
    settings_input(
        &mut app,
        "description",
        &[Key::Enter, Key::End, Key::Char('!')],
    );
    send.send(result).unwrap();
    assert!(app.poll_rule_save());
    finish_autosaves(&mut app);
    app.tick(Duration::ZERO, AnimationSettings::default());
    assert_setting(&mut app, "Start instance", true);
    assert_setting(&mut app, "Active", false);
    assert_eq!(
        draft.borrow().rule.definition.description,
        "editable event handler!"
    );
    assert!(draft.borrow().saved.definition.start_instance);
}

#[test]
fn settings_toggles_reject_dirty_unsaved_and_bulk_pending_fields() {
    for blocked in ["dirty", "unsaved", "bulk"] {
        let mut app = editor(false);
        show_settings(&mut app);
        let draft = app.rule_editor.as_ref().unwrap().clone();
        match blocked {
            "dirty" => draft.borrow_mut().dirty = true,
            "unsaved" => draft.borrow_mut().rule.definition.description = "unsaved".into(),
            "bulk" => app.rule_saves.push(saved_rule(&app)),
            _ => unreachable!(),
        }
        let original = draft.borrow().rule.clone();
        for (slot, label, checked) in [
            ("enabled", "Active", false),
            ("start-instance", "Start instance", true),
            ("focus-pane", "Focus new pane", true),
        ] {
            settings_input(&mut app, slot, &[Key::Char(' ')]);
            app.tick(Duration::ZERO, AnimationSettings::default());
            assert!(app.rule_save.is_none());
            assert!(draft.borrow().pending_toggle.is_none());
            assert_eq!(draft.borrow().rule, original);
            assert_setting(&mut app, label, checked);
        }
    }
}

fn reopen_pending_toggle(slot: &str, edit: bool) {
    let mut app = editor(slot != "enabled");
    show_settings(&mut app);
    let draft = app.rule_editor.as_ref().unwrap().clone();
    let original = draft.borrow().saved.clone();
    settings_input(&mut app, slot, &[Key::Char(' ')]);
    let (send, result) = delay_save(&mut app);
    assert!(result.is_ok());
    app.handle_message(Msg::Close, &mut EventCtx::default());
    app.open_rule(original.clone(), &mut EventCtx::default());
    assert!(Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &draft));
    show_settings(&mut app);
    app.tick(Duration::ZERO, AnimationSettings::default());
    assert_setting(&mut app, "Active", true);
    assert_setting(&mut app, "Start instance", slot != "start-instance");
    assert_setting(&mut app, "Focus new pane", slot != "focus-pane");
    if edit {
        settings_input(
            &mut app,
            "description",
            &[Key::Enter, Key::End, Key::Char('!')],
        );
        app.handle_message(Msg::Close, &mut EventCtx::default());
        app.open_rule(original.clone(), &mut EventCtx::default());
        assert!(Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &draft));
        show_settings(&mut app);
        assert!(render(&mut app, 130).1.contains("editable event handler!"));
    }
    send.send(result).unwrap();
    assert!(app.poll_rule_save());
    if edit {
        assert!(app.rule_autosave.is_some());
        app.handle_message(Msg::Close, &mut EventCtx::default());
        app.open_rule(original, &mut EventCtx::default());
        assert!(Rc::ptr_eq(app.rule_editor.as_ref().unwrap(), &draft));
        show_settings(&mut app);
        assert!(render(&mut app, 130).1.contains("editable event handler!"));
    }
    finish_autosaves(&mut app);
    app.tick(Duration::ZERO, AnimationSettings::default());
    let saved = saved_rule(&app);
    assert_eq!(saved.revision, if edit { 4 } else { 2 });
    assert_eq!(saved.definition.enabled, !edit);
    assert_eq!(saved.definition.start_instance, slot != "start-instance");
    assert_eq!(saved.definition.focus_pane, slot != "focus-pane");
    assert_eq!(
        saved.definition.description,
        if edit {
            "editable event handler!"
        } else {
            "editable event handler"
        }
    );
    assert_eq!(
        saved.zellij_session,
        if slot == "enabled" {
            "active-rule-session"
        } else {
            "main"
        }
    );
    assert_eq!(draft.borrow().rule, saved);
    assert_eq!(draft.borrow().saved, saved);
    assert_setting(&mut app, "Active", !edit);
    assert_setting(&mut app, "Start instance", slot != "start-instance");
    assert_setting(&mut app, "Focus new pane", slot != "focus-pane");
}

#[test]
fn reopening_during_activation_preserves_draft_and_pauses_for_text_edits() {
    for edit in [false, true] {
        reopen_pending_toggle("enabled", edit);
    }
}

#[test]
fn reopening_during_start_instance_save_preserves_draft_and_selected_setting() {
    for edit in [false, true] {
        reopen_pending_toggle("start-instance", edit);
    }
}

#[test]
fn reopening_during_focus_pane_save_preserves_the_selection_and_text_edits() {
    for edit in [false, true] {
        reopen_pending_toggle("focus-pane", edit);
    }
}

#[test]
fn opening_a_different_rule_keeps_pending_toggle_completion_isolated() {
    let mut app = editor(false);
    show_settings(&mut app);
    let pending = app.rule_editor.as_ref().unwrap().clone();
    settings_input(&mut app, "enabled", &[Key::Char(' ')]);
    let (send, result) = delay_save(&mut app);
    app.handle_message(Msg::Close, &mut EventCtx::default());
    let other = rule("other");
    app.open_rule(other.clone(), &mut EventCtx::default());
    let editor = app.rule_editor.as_ref().unwrap().clone();
    assert!(!Rc::ptr_eq(&editor, &pending));
    send.send(result).unwrap();
    assert!(app.poll_rule_save());
    assert_eq!(pending.borrow().rule, saved_rule(&app));
    assert_eq!(editor.borrow().rule, other);
    assert_eq!(editor.borrow().saved, other);
}

#[test]
fn field_input_autosaves_without_closing_or_replacing_the_editor() {
    let mut app = editor(false);
    show_settings(&mut app);
    let (layout, _) = render(&mut app, 130);
    let toggle = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "start-instance")
        })
        .unwrap();
    app.dispatch_focus(toggle, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::default();
    app.dispatch_event(
        &EventRoute::new(toggle.path.clone()),
        &TuiEvent::Key(Key::Char(' ').into()),
        &mut ctx,
    );
    for message in ctx.drain_messages() {
        app.handle_message(message, &mut EventCtx::default());
    }
    finish_autosaves(&mut app);
    assert!(!saved_rule(&app).definition.start_instance);
    app.dispatch_focus(toggle, false, &mut tuicore::FocusCtx::default());
    let input = layout
        .focus_targets()
        .iter()
        .find(|target| {
            target
                .path
                .keys()
                .iter()
                .any(|key| key.as_str() == "description")
        })
        .unwrap();
    app.dispatch_focus(input, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::default();
    for key in [Key::Enter, Key::End, Key::Char('!')] {
        app.dispatch_event(
            &EventRoute::new(input.path.clone()),
            &TuiEvent::Key(key.into()),
            &mut ctx,
        );
        for message in ctx.drain_messages() {
            app.handle_message(message, &mut EventCtx::default());
        }
    }
    finish_autosaves(&mut app);
    assert_eq!(
        saved_rule(&app).definition.description,
        "editable event handler!"
    );
    assert!(app.view.is_active());
    assert!(render(&mut app, 130).1.contains("editable event handler!"));
    app.dispatch_focus(input, false, &mut tuicore::FocusCtx::default());
    settings_input(
        &mut app,
        "variant",
        &[
            Key::Enter,
            Key::Char('h'),
            Key::Char('i'),
            Key::Char('g'),
            Key::Char('h'),
            Key::Esc,
        ],
    );
    finish_autosaves(&mut app);
    assert_eq!(saved_rule(&app).definition.variant.as_deref(), Some("high"));
    settings_input(
        &mut app,
        "variant",
        &[
            Key::Enter,
            Key::Home,
            Key::Delete,
            Key::Delete,
            Key::Delete,
            Key::Delete,
            Key::Esc,
        ],
    );
    finish_autosaves(&mut app);
    assert_eq!(saved_rule(&app).definition.variant, None);
}

#[test]
fn rapid_edits_save_the_latest_fields_and_survive_dialog_close() {
    let mut app = editor(false);
    let draft = app.rule_editor.as_ref().unwrap().clone();
    draft.borrow_mut().rule.definition.description = "first edit".into();
    app.handle_message(
        Msg::RuleDraftChanged(draft.clone()),
        &mut EventCtx::default(),
    );
    {
        let mut draft = draft.borrow_mut();
        draft.rule.definition.description = "latest edit".into();
        draft.rule.definition.script = "fn matches(event) { false }".into();
        draft.rule.definition.model = "custom/model".into();
        draft.rule.definition.variant = Some("fast".into());
        draft.rule.definition.initial_prompt = "Read {{event}}".into();
    }
    app.handle_message(Msg::RuleDraftChanged(draft), &mut EventCtx::default());
    app.handle_message(Msg::Close, &mut EventCtx::default());
    finish_autosaves(&mut app);
    let saved = saved_rule(&app);
    assert_eq!(saved.revision, 3);
    assert_eq!(saved.definition.description, "latest edit");
    assert_eq!(saved.definition.script, "fn matches(event) { false }");
    assert_eq!(saved.definition.model, "custom/model");
    assert_eq!(saved.definition.variant.as_deref(), Some("fast"));
    assert_eq!(saved.definition.initial_prompt, "Read {{event}}");
    assert!(!app.view.is_active());
    show_rules(&mut app, vec![saved]);
    assert!(render(&mut app, 160).1.contains("custom/model · fast · 󰒋"));
}

#[test]
fn editing_pauses_an_enabled_rule_and_preserves_valid_values_until_input_is_corrected() {
    let mut app = editor(true);
    let original = saved_rule(&app);
    let draft = app.rule_editor.as_ref().unwrap().clone();
    draft.borrow_mut().rule.definition.script = "fn matches(event) {".into();
    app.handle_message(
        Msg::RuleDraftChanged(draft.clone()),
        &mut EventCtx::default(),
    );
    finish_autosaves(&mut app);
    let paused = saved_rule(&app);
    assert!(!paused.definition.enabled);
    assert_eq!(paused.definition.script, original.definition.script);
    assert!(app.view.is_active());
    let notice = app.notifications.center().history().last().unwrap();
    assert_eq!(notice.title(), "Cannot save rule");
    assert_eq!(notice.kind(), tuicore::NotificationKind::Error);
    app.handle_message(
        Msg::RuleDraftEnabled(draft.clone(), true),
        &mut EventCtx::default(),
    );
    assert!(app.rule_save.is_none());
    assert_eq!(saved_rule(&app), paused);
    draft.borrow_mut().rule.definition.script = "fn matches(event) { false }".into();
    app.handle_message(
        Msg::RuleDraftChanged(draft.clone()),
        &mut EventCtx::default(),
    );
    finish_autosaves(&mut app);
    let saved = saved_rule(&app);
    assert_eq!(saved.definition.script, "fn matches(event) { false }");
    assert!(!saved.definition.enabled);
    assert!(app.view.is_active());
    app.handle_message(
        Msg::RuleDraftEnabled(draft.clone(), true),
        &mut EventCtx::default(),
    );
    assert!(app.rule_save.is_some());
    finish_autosaves(&mut app);
    assert!(saved_rule(&app).definition.enabled);
    assert!(draft.borrow().rule.definition.enabled);
    assert!(app.view.is_active());
    assert_eq!(
        app.notifications.center().history().last().unwrap().title(),
        "Rule activated"
    );
    app.handle_message(
        Msg::RuleDraftEnabled(draft.clone(), false),
        &mut EventCtx::default(),
    );
    finish_autosaves(&mut app);
    assert!(!saved_rule(&app).definition.enabled);
    assert!(!draft.borrow().rule.definition.enabled);
    assert!(app.view.is_active());
    assert_eq!(
        app.notifications.center().history().last().unwrap().title(),
        "Rule deactivated"
    );
}

#[test]
fn rejected_rule_activation_reports_an_error_and_keeps_the_saved_state() {
    let mut app = editor(false);
    let mut stale = saved_rule(&app);
    let current = app
        .service
        .save_rule(
            stale.definition.clone(),
            Some(stale.revision),
            Some(stale.zellij_session.clone()),
            false,
        )
        .blocking_recv()
        .unwrap()
        .unwrap();
    stale.definition.enabled = true;
    app.handle_message(Msg::SaveRule(Box::new(stale)), &mut EventCtx::default());
    finish_autosaves(&mut app);
    assert_eq!(saved_rule(&app), current);
    let notice = app.notifications.center().history().last().unwrap();
    assert_eq!(notice.title(), "Cannot save rule");
    assert_eq!(notice.kind(), tuicore::NotificationKind::Error);
}
