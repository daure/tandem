use super::super::rows::Tone;
use super::*;
use crate::store::opencode::{Activity, Client, Pane, Session, Snapshot};

fn external_observation() -> Snapshot {
    let pane = Pane {
        session: "main".into(),
        id: 17,
        tab_id: 4,
        tab_name: "ledger".into(),
    };
    Snapshot {
        sessions: vec![
            Session {
                id: "ses_external".into(),
                title: "Add CSV export".into(),
                directory: "/work/ledger".into(),
                activity: Activity::Busy,
                panes: vec![pane],
                last_question: Some("Implement the download endpoint".into()),
                question_observed: true,
                ..Default::default()
            },
            Session {
                id: "ses_external_saved".into(),
                title: "Investigate rounding".into(),
                directory: "/work/ledger".into(),
                activity: Activity::Idle,
                ..Default::default()
            },
        ],
        clients: vec![Client {
            title: "OpenCode".into(),
            directory: "/tmp/prototype".into(),
            server: "http://127.0.0.1:4199".into(),
            pane: Pane {
                session: "main".into(),
                id: 18,
                tab_id: 5,
                tab_name: "prototype".into(),
            },
            stale: false,
        }],
        error: None,
    }
}

#[test]
fn overview_places_other_opencode_workspaces_before_templates() {
    tuicore::init();
    let mut projected = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut projected, &external_observation(), false);

    assert_eq!(projected[0].id, "opencode-workspaces");
    assert_eq!(projected[0].label, "Other OpenCode workspaces");
    assert!(matches!(
        projected[0].opencode,
        Some(super::super::opencode::Target::Workspace)
    ));
    let ledger = projected
        .iter()
        .find(|row| row.id == "opencode-workspace:/work/ledger")
        .unwrap();
    assert_eq!(ledger.parent.as_deref(), Some("opencode-workspaces"));
    assert_eq!(ledger.label, "/work/ledger");
    assert_eq!(ledger.template_capabilities, " external");
    assert_eq!(
        ledger.text("⠋", None).lines[0]
            .spans
            .last()
            .unwrap()
            .style
            .fg,
        Some(tuicore::theme().muted_fg())
    );
    let external = projected
        .iter()
        .find(|row| row.id.ends_with(":ses_external"))
        .unwrap();
    assert_eq!(external.parent.as_deref(), Some(ledger.id.as_str()));
    assert_eq!(
        external
            .opencode
            .as_ref()
            .and_then(super::super::opencode::Target::session_action),
        Some((true, false))
    );
    assert!(
        projected
            .iter()
            .all(|row| !row.id.ends_with(":ses_external_saved"))
    );
    assert!(
        projected
            .iter()
            .position(|row| row.id == "opencode-workspaces")
            < projected.iter().position(rows::Row::is_template)
    );

    let mut with_history = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut with_history, &external_observation(), true);
    assert!(
        with_history
            .iter()
            .any(|row| row.id.ends_with(":ses_external_saved"))
    );
}

#[test]
fn overview_expansion_stops_at_external_workspace_rows() {
    use crate::app::instances::{self, Instances};

    tuicore::init();
    let mut projected = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut projected, &external_observation(), false);
    let state = instances::state(projected);
    let mut tree = Instances::new(state.clone());
    let area = Rect::new(0, 0, 130, 40);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let render = |tree: &mut Instances, terminal: &mut Terminal<TestBackend>| {
        tree.layout(area, &mut tuicore::LayoutCtx::new());
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                tree.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        rendered_lines(terminal, area).join("\n")
    };

    let startup = render(&mut tree, &mut terminal);
    assert!(startup.contains("Other OpenCode workspaces"), "{startup}");
    assert!(startup.contains("/work/ledger"), "{startup}");
    assert!(!startup.contains("Add CSV export"), "{startup}");

    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('z'))), &mut ctx);
    assert!(!render(&mut tree, &mut terminal).contains("/work/ledger"));
    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('z'))), &mut ctx);
    let expanded = render(&mut tree, &mut terminal);
    assert!(expanded.contains("/work/ledger"), "{expanded}");
    assert!(!expanded.contains("Add CSV export"), "{expanded}");

    instances::set_attached_sessions_only(&state, true);
    tree.event(
        &TuiEvent::Hotkey(HotkeyEvent::Commit("shift+h".into())),
        &mut ctx,
    );
    let home = render(&mut tree, &mut terminal);
    assert!(home.contains("/work/ledger"), "{home}");
    assert!(home.contains("Add CSV export"), "{home}");
}

#[test]
fn agent_view_startup_and_shift_h_expand_external_workspaces() {
    use crate::app::instances::{self, Instances};

    tuicore::init();
    let state = instances::state(Vec::new());
    instances::set_attached_sessions_only(&state, true);
    let mut tree = Instances::new(state.clone());
    let area = Rect::new(0, 0, 130, 40);
    tree.layout(area, &mut tuicore::LayoutCtx::new());
    instances::replace_rows(
        &state,
        super::super::opencode::attached_rows(
            rows::from_snapshot(&snapshot()),
            &external_observation(),
        ),
    );
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let render = |tree: &mut Instances, terminal: &mut Terminal<TestBackend>| {
        tree.layout(area, &mut tuicore::LayoutCtx::new());
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                tree.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        rendered_lines(terminal, area).join("\n")
    };
    let startup = render(&mut tree, &mut terminal);

    assert!(startup.contains("/work/ledger"), "{startup}");
    assert!(startup.contains("Add CSV export"), "{startup}");

    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('z'))), &mut ctx);
    assert!(!render(&mut tree, &mut terminal).contains("Add CSV export"));
    tree.event(
        &TuiEvent::Hotkey(HotkeyEvent::Commit("shift+h".into())),
        &mut ctx,
    );
    assert!(render(&mut tree, &mut terminal).contains("Add CSV export"));
    tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('z'))), &mut ctx);
    assert!(!render(&mut tree, &mut terminal).contains("Add CSV export"));
}

#[test]
fn switching_from_agents_to_templates_collapses_external_workspaces() {
    use crate::app::instances::{self, Instances};

    tuicore::init();
    let state = instances::state(super::super::opencode::attached_rows(
        rows::from_snapshot(&snapshot()),
        &external_observation(),
    ));
    instances::set_attached_sessions_only(&state, true);
    let mut tree = Instances::new(state.clone());
    let area = Rect::new(0, 0, 130, 40);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let render = |tree: &mut Instances, terminal: &mut Terminal<TestBackend>| {
        tree.layout(area, &mut tuicore::LayoutCtx::new());
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                tree.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        rendered_lines(terminal, area).join("\n")
    };
    assert!(render(&mut tree, &mut terminal).contains("Add CSV export"));

    let mut templates = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut templates, &external_observation(), false);
    instances::set_attached_sessions_only(&state, false);
    instances::replace_rows(&state, templates);
    let switched = render(&mut tree, &mut terminal);

    assert!(switched.contains("Other OpenCode workspaces"), "{switched}");
    assert!(switched.contains("/work/ledger"), "{switched}");
    assert!(!switched.contains("Add CSV export"), "{switched}");
}

#[test]
fn attached_view_promotes_external_directories_to_workspace_rows() {
    tuicore::init();
    let projected = super::super::opencode::attached_rows(
        rows::from_snapshot(&snapshot()),
        &external_observation(),
    );

    let ledger = projected
        .iter()
        .find(|row| row.id == "opencode-workspace:/work/ledger")
        .unwrap();
    assert_eq!(ledger.parent, None);
    assert_eq!(ledger.label, "ledger\n/work/ledger");
    assert_eq!(ledger.template_capabilities, " external");
    assert_eq!(
        ledger.text("⠋", None).lines[0]
            .spans
            .last()
            .unwrap()
            .style
            .fg,
        Some(tuicore::theme().muted_fg())
    );
    assert!(
        projected
            .iter()
            .any(|row| row.parent.as_deref() == Some(ledger.id.as_str())
                && row.id.ends_with(":ses_external"))
    );
    assert!(
        projected
            .iter()
            .all(|row| !row.id.ends_with(":ses_external_saved"))
    );
    let prototype = projected
        .iter()
        .find(|row| row.id == "opencode-workspace:/tmp/prototype")
        .unwrap();
    assert_eq!(prototype.parent, None);
    assert!(projected.iter().any(|row| {
        row.parent.as_deref() == Some(prototype.id.as_str())
            && matches!(
                row.opencode,
                Some(super::super::opencode::Target::Client { .. })
            )
    }));
}

#[test]
fn session_views_show_twenty_recent_sessions_per_directory_then_a_muted_more_row() {
    tuicore::init();
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let mut observation = Snapshot::default();
    for directory in ["/tmp/workspaces/review/repo", "/work/external"] {
        for updated in 0..22 {
            observation.sessions.push(Session {
                id: format!("ses_{}_{updated}", directory.replace('/', "_")),
                title: format!("Conversation {updated}"),
                directory: directory.into(),
                activity: Activity::Busy,
                panes: vec![pane.clone()],
                updated,
                ..Default::default()
            });
        }
    }

    let assert_window = |projected: &[rows::Row], parent: &str, scope: &str, directory: &str| {
        let children = projected
            .iter()
            .filter(|row| row.parent.as_deref() == Some(parent))
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 21);
        let sessions = children
            .iter()
            .filter(|row| {
                matches!(
                    row.opencode,
                    Some(super::super::opencode::Target::Session { .. })
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(sessions.len(), 20);
        for omitted in 0..2 {
            let id = format!("ses_{}_{omitted}", directory.replace('/', "_"));
            assert!(sessions.iter().all(|row| !row.id.ends_with(&id)));
        }
        let more = children.last().unwrap();
        assert_eq!(more.id, format!("opencode-more:{scope}"));
        assert_eq!(more.label, "(see more sessions in opencode)");
        assert_eq!(more.tone, Tone::Muted);
        assert!(more.informational);
        assert_eq!(
            more.text("", None).lines[0].spans[0].style.fg,
            Some(tuicore::theme().muted_fg())
        );
    };

    let mut overview = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut overview, &observation, false);
    assert_window(
        &overview,
        "sessions:review",
        "review",
        "/tmp/workspaces/review/repo",
    );
    assert_window(
        &overview,
        "opencode-workspace:/work/external",
        "external:/work/external",
        "/work/external",
    );

    let attached =
        super::super::opencode::attached_rows(rows::from_snapshot(&snapshot()), &observation);
    assert_window(
        &attached,
        "instance:review",
        "review",
        "/tmp/workspaces/review/repo",
    );
    assert_window(
        &attached,
        "opencode-workspace:/work/external",
        "external:/work/external",
        "/work/external",
    );
}

#[test]
fn session_views_put_recent_completions_first_and_recent_starts_last() {
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let session = |id: &str, activity, updated, started| Session {
        id: id.into(),
        title: id.into(),
        directory: "/tmp/workspaces/review".into(),
        activity,
        activity_started_at_milliseconds: started,
        panes: vec![pane.clone()],
        updated,
        ..Default::default()
    };
    let observation = Snapshot {
        sessions: vec![
            session("ses_busy_new", Activity::Busy, 400, Some(200)),
            session("ses_done_old", Activity::Idle, 300, None),
            session("ses_busy_old", Activity::Busy, 500, Some(100)),
            session("ses_done_new", Activity::Idle, 600, None),
        ],
        ..Default::default()
    };
    let expected = [
        "ses_done_new".to_owned(),
        "ses_done_old".to_owned(),
        "ses_busy_old".to_owned(),
        "ses_busy_new".to_owned(),
    ];
    let ids = |rows: &[rows::Row]| {
        rows.iter()
            .filter_map(|row| match row.opencode.as_ref()? {
                super::super::opencode::Target::Session { id, .. } => Some(id.clone()),
                super::super::opencode::Target::Workspace
                | super::super::opencode::Target::Client { .. } => None,
            })
            .collect::<Vec<_>>()
    };

    let mut overview = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut overview, &observation, false);
    assert_eq!(ids(&overview), expected);

    let attached =
        super::super::opencode::attached_rows(rows::from_snapshot(&snapshot()), &observation);
    assert_eq!(ids(&attached), expected);
}

#[test]
fn disabling_integration_removes_rows_and_counts_and_clears_the_cached_observation() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: vec![Session {
            id: "ses_busy".into(),
            title: "Working".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Busy,
            ..Default::default()
        }],
        clients: Vec::new(),
        error: None,
    });
    app.update_snapshot(snapshot());
    super::super::instances::set_highlighted(
        &app.instances,
        Some("opencode:review:ses_busy".into()),
    );
    assert!(app.selected().is_some());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.handle_message(Msg::SetOpencodeIntegration(false), &mut ctx);
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(app.settings_save.take().unwrap())
        .unwrap()
        .unwrap();
    app.update_snapshot(snapshot());
    assert!(app.opencode_snapshot.sessions.is_empty());
    assert!(app.selected().is_none());
    let mut rows = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut rows, &app.opencode_snapshot, false);
    assert!(rows.iter().all(|row| row.opencode.is_none()));
    assert!(rows.iter().all(|row| {
        !row.status_detail
            .as_deref()
            .unwrap_or_default()
            .contains("OpenCode")
    }));
}

#[test]
fn child_rows_show_four_states_with_overlapping_counts_and_explicit_history() {
    tuicore::init();
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let mut observation = Snapshot::default();
    for (index, (activity, attached)) in [
        (Activity::Busy, true),
        (Activity::Idle, true),
        (Activity::Busy, false),
        (Activity::Idle, false),
    ]
    .into_iter()
    .enumerate()
    {
        observation.sessions.push(Session {
            id: format!("ses_{index}"),
            title: format!("Conversation {index}"),
            directory: "/tmp/workspaces/review/repo".into(),
            activity,
            activity_elapsed_milliseconds: Some(if index % 2 == 0 { 29_000 } else { 114_000 }),
            panes: if attached { vec![pane.clone()] } else { vec![] },
            last_question: Some(format!("Question {index}")),
            question_observed: true,
            ..Default::default()
        });
    }
    let mut rows = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut rows, &observation, false);
    assert_eq!(
        rows.iter()
            .find(|row| row.instance.is_some())
            .unwrap()
            .status_detail,
        None
    );
    assert_eq!(
        rows.iter()
            .filter(|row| matches!(
                row.opencode,
                Some(super::super::opencode::Target::Session { .. })
            ))
            .count(),
        3
    );
    let sessions = rows.iter().find(|row| row.id == "sessions:review").unwrap();
    assert!(sessions.loading);
    assert_eq!(sessions.tone, Tone::Info);
    assert_eq!(sessions.text("⠧", None).to_string(), "⠧ Sessions");
    let mut rows = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut rows, &observation, true);
    for (id, tone, activity, activity_tone, elapsed, elapsed_tone) in [
        ("ses_0", Tone::Info, "⠋", Tone::Info, "29s", Tone::Info),
        (
            "ses_1",
            Tone::Success,
            "",
            Tone::Success,
            "1m54s",
            Tone::Muted,
        ),
        ("ses_2", Tone::Info, "⠋", Tone::Info, "29s", Tone::Info),
        ("ses_3", Tone::Muted, "", Tone::Muted, "1m54s", Tone::Muted),
    ] {
        let row = rows
            .iter()
            .find(|row| row.id == format!("opencode:review:{id}"))
            .unwrap();
        assert_eq!(row.icon, "󰚩");
        assert_eq!(row.tone, tone);
        assert!(row.status.is_none());
        assert_eq!(
            row.text("⠋", None).lines[1].spans[0].content,
            format!("{activity} ")
        );
        assert_eq!(row.secondary_tone, activity_tone);
        assert_eq!(row.status_detail.as_deref(), Some(elapsed));
        assert_eq!(row.detail_tone, elapsed_tone);
        assert_eq!(
            row.text("", None).lines[0].to_string(),
            format!("󰚩 Conversation {} · {elapsed}", &id[4..])
        );
        assert!(row.label.ends_with(&format!("\nQuestion {}", &id[4..])));
        assert!(!row.label.contains(id));
    }
    assert!(
        rows.iter()
            .filter(|row| row.opencode.is_some())
            .all(|row| !row.can_stop && !row.can_restart && row.instance.is_none())
    );
}

#[test]
fn session_timers_share_a_display_beat_across_refreshes_and_stop_when_idle() {
    use crate::app::instances::{self, Instances};
    use std::time::Duration;

    tuicore::init();
    let mut observation = Snapshot {
        sessions: vec![Session {
            id: "ses_timer".into(),
            title: "Quick ping".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Busy,
            activity_started_at_milliseconds: Some(1_000),
            activity_elapsed_milliseconds: Some(320),
            context_tokens: Some(83_600),
            context_limit: Some(272_000),
            last_question: Some("Sleep for 30 seconds".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut second_session = observation.sessions[0].clone();
    second_session.id = "ses_second".into();
    second_session.title = "Second ping".into();
    second_session.activity_started_at_milliseconds = Some(740);
    second_session.activity_elapsed_milliseconds = Some(580);
    observation.sessions.push(second_session);
    let make_rows = |observation: &Snapshot| {
        let mut rows = rows::from_snapshot(&snapshot());
        super::super::opencode::append_rows(&mut rows, observation, true);
        rows
    };
    let state = instances::state(make_rows(&observation));
    let mut tree = Instances::new(state.clone());
    tree.expand_for_tests("instance:review");
    tree.expand_for_tests("sessions:review");
    let area = Rect::new(0, 0, 130, 30);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let render = |tree: &mut Instances, terminal: &mut Terminal<TestBackend>| {
        tree.layout(area, &mut tuicore::LayoutCtx::new());
        terminal
            .draw(|frame| {
                let mut ctx = RenderCtx::new();
                tree.render(frame, area, &mut ctx);
                ctx.flush(frame);
            })
            .unwrap();
        rendered_lines(terminal, area).join("\n")
    };
    assert!(render(&mut tree, &mut terminal).contains("⠋ Sleep for 30 seconds"));
    tree.tick(Duration::from_millis(80), AnimationSettings::default());
    assert!(render(&mut tree, &mut terminal).contains("⠙ Sleep for 30 seconds"));
    tree.tick(Duration::from_millis(920), AnimationSettings::default());
    for second in 1..=4 {
        let text = render(&mut tree, &mut terminal);
        assert!(
            text.contains(&format!("Quick ping · {second}s · 84k/272k (31%)")),
            "{text}"
        );
        assert!(
            text.contains(&format!("Second ping · {second}s · 84k/272k (31%)")),
            "{text}"
        );
        tree.tick(Duration::from_millis(420), AnimationSettings::default());
        let text = render(&mut tree, &mut terminal);
        assert!(
            text.contains(&format!("Quick ping · {second}s · 84k/272k (31%)")),
            "{text}"
        );
        assert!(
            text.contains(&format!("Second ping · {second}s · 84k/272k (31%)")),
            "{text}"
        );
        observation.sessions[0].activity_elapsed_milliseconds = Some(second * 1_000 + 740);
        observation.sessions[1].activity_elapsed_milliseconds = Some(second * 1_000 + 1_000);
        instances::replace_rows(&state, make_rows(&observation));
        let text = render(&mut tree, &mut terminal);
        assert!(
            text.contains(&format!("Quick ping · {second}s · 84k/272k (31%)")),
            "{text}"
        );
        assert!(
            text.contains(&format!("Second ping · {second}s · 84k/272k (31%)")),
            "{text}"
        );
        tree.tick(Duration::from_millis(580), AnimationSettings::default());
    }
    observation.sessions[0].activity = Activity::Idle;
    observation.sessions[0].activity_started_at_milliseconds = None;
    observation.sessions[0].activity_elapsed_milliseconds = Some(5_000);
    instances::replace_rows(&state, make_rows(&observation));
    tree.tick(Duration::from_secs(10), AnimationSettings::default());
    let text = render(&mut tree, &mut terminal);
    assert!(text.contains("Quick ping · 5s · 84k/272k (31%)"), "{text}");
    assert!(text.contains(" Sleep for 30 seconds"), "{text}");
}

#[test]
fn attached_client_without_a_conversation_appears_in_the_sessions_group() {
    let observation = Snapshot {
        clients: vec![Client {
            title: "OpenCode".into(),
            directory: "/tmp/workspaces/review".into(),
            server: "http://127.0.0.1:4199".into(),
            pane: Pane {
                session: "main".into(),
                id: 12,
                tab_id: 4,
                tab_name: "review".into(),
            },
            stale: false,
        }],
        ..Default::default()
    };
    let mut rows = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut rows, &observation, false);
    let instance = rows.iter().find(|row| row.id == "instance:review").unwrap();
    assert_eq!(instance.status_detail, None);
    let client = rows
        .iter()
        .find(|row| row.id == "opencode-client:review:main:12")
        .unwrap();
    assert_eq!(client.parent.as_deref(), Some("sessions:review"));
    assert_eq!(client.label, "OpenCode\n(new session)");
    assert!(client.status.is_none());
    assert_eq!(client.tone, super::super::rows::Tone::Success);
    assert_eq!(client.secondary_icon, "");
    assert_eq!(client.secondary_tone, super::super::rows::Tone::Subtle);
    let text = client.text("", Some(80));
    assert!(
        text.lines[1]
            .spans
            .iter()
            .all(|span| span.style.fg == Some(tuicore::theme().subtle_fg()))
    );
    assert!(client.opencode.is_none());
}

#[test]
fn latest_question_uses_ascii_ellipsis_at_the_available_width() {
    let mut observation = Snapshot::default();
    observation.sessions.push(Session {
        id: "ses_long".into(),
        title: "Long conversation".into(),
        directory: "/tmp/workspaces/review".into(),
        activity: Activity::Idle,
        panes: vec![Pane {
            session: "main".into(),
            id: 7,
            tab_id: 4,
            tab_name: "review".into(),
        }],
        last_question: Some("Explain every part of this particularly long question".into()),
        question_observed: true,
        ..Default::default()
    });
    let mut rows = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut rows, &observation, false);
    let row = rows
        .iter()
        .find(|row| row.id == "opencode:review:ses_long")
        .unwrap();
    let rendered = row.text("", Some(24)).lines[1].to_string();
    assert!(rendered.ends_with("..."), "{rendered}");
    assert!(!rendered.contains('…'), "{rendered}");
    assert!(ratatui::text::Line::from(rendered).width() <= 24);
}

#[test]
fn instance_children_group_opencode_sessions_before_setup_and_services() {
    let mut inventory = snapshot();
    let mut setup = inventory.instances[0].services[0].clone();
    setup.name = "repo-sync".into();
    setup.one_shot = true;
    setup.status = "exited 0".into();
    setup.port = None;
    setup.url = None;
    inventory.instances[0].services.push(setup);
    let observation = Snapshot {
        sessions: vec![Session {
            id: "ses_review".into(),
            title: "Review".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Idle,
            panes: vec![Pane {
                session: "main".into(),
                id: 7,
                tab_id: 4,
                tab_name: "review".into(),
            }],
            last_question: Some("Review the changes".into()),
            question_observed: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut rows = rows::from_snapshot(&inventory);
    super::super::opencode::append_rows(&mut rows, &observation, false);
    assert_eq!(
        rows.iter()
            .filter(|row| row.parent.as_deref() == Some("instance:review"))
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["sessions:review", "setup:review", "services:review"]
    );
    let sessions = rows.iter().find(|row| row.id == "sessions:review").unwrap();
    assert_eq!(sessions.text("", None).to_string(), "󰚩 Sessions");
    assert_eq!(sessions.tone, Tone::Success);
    assert!(!sessions.loading);
    assert_eq!(sessions.height(), 1);
    assert_eq!(
        rows.iter()
            .find(|row| row.id == "opencode:review:ses_review")
            .unwrap()
            .parent
            .as_deref(),
        Some("sessions:review")
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.id == "service:review:web")
            .unwrap()
            .parent
            .as_deref(),
        Some("services:review")
    );
}

#[test]
fn sessions_group_is_muted_for_history_only_and_green_with_an_attached_session() {
    tuicore::init();
    let observation = Snapshot {
        sessions: vec![Session {
            id: "ses_saved".into(),
            title: "Saved conversation".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Idle,
            last_question: Some("Old question".into()),
            question_observed: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut rows = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut rows, &observation, true);
    let sessions = rows.iter().find(|row| row.id == "sessions:review").unwrap();
    assert_eq!(sessions.text("", None).to_string(), "󰚩 Sessions");
    assert_eq!(sessions.tone, Tone::Muted);
    assert!(!sessions.loading);
    assert_eq!(
        sessions.text("", None).lines[0].spans[0].style.fg,
        Some(tuicore::theme().muted_fg())
    );

    let mut mixed = observation;
    mixed.sessions.push(Session {
        id: "ses_attached".into(),
        title: "Attached conversation".into(),
        directory: "/tmp/workspaces/review".into(),
        activity: Activity::Idle,
        panes: vec![Pane {
            session: "main".into(),
            id: 7,
            tab_id: 4,
            tab_name: "review".into(),
        }],
        ..Default::default()
    });
    let mut rows = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut rows, &mixed, true);
    let sessions = rows.iter().find(|row| row.id == "sessions:review").unwrap();
    assert_eq!(sessions.tone, Tone::Success);
    assert!(!sessions.loading);
}

#[test]
fn multiple_panes_share_one_session_row_and_have_individual_navigation_targets() {
    let mut observation = Snapshot::default();
    observation.sessions.push(Session {
        id: "ses_one".into(),
        title: "Review".into(),
        directory: "/tmp/workspaces/review".into(),
        activity: Activity::Idle,
        panes: vec![
            Pane {
                session: "main".into(),
                id: 7,
                tab_id: 4,
                tab_name: "review".into(),
            },
            Pane {
                session: "main".into(),
                id: 8,
                tab_id: 4,
                tab_name: "review".into(),
            },
        ],
        ..Default::default()
    });
    let mut rows = rows::from_snapshot(&snapshot());
    super::super::opencode::append_rows(&mut rows, &observation, false);
    assert_eq!(rows[1].status_detail, None);
    assert_eq!(
        rows.iter()
            .filter(|row| row.parent.as_deref() == Some("opencode:review:ses_one"))
            .count(),
        2
    );
    assert!(matches!(
        rows.iter()
            .find(|row| row.id == "opencode:review:ses_one")
            .and_then(|row| row.opencode.as_ref()),
        Some(super::super::opencode::Target::Session {
            pane: Some(Pane { id: 7, .. }),
            ..
        })
    ));
}

#[test]
fn session_menu_and_shortcut_open_or_goto_the_panel() {
    tuicore::init();
    for attached in [false, true] {
        let pane = Pane {
            session: "main".into(),
            id: 7,
            tab_id: 4,
            tab_name: "review".into(),
        };
        let mut app = root(AppService::for_tests());
        app.service.set_opencode_snapshot_for_tests(Snapshot {
            sessions: vec![Session {
                id: "ses_review".into(),
                title: "Review".into(),
                directory: "/tmp/workspaces/review".into(),
                activity: Activity::Busy,
                panes: attached.then_some(pane).into_iter().collect(),
                ..Default::default()
            }],
            ..Default::default()
        });
        app.update_snapshot(snapshot());
        let area = Rect::new(0, 0, 130, 40);
        let mut ctx = EventCtx::new(AnimationSettings::default());
        if attached {
            app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut ctx);
        }
        app.layout(area, &mut tuicore::LayoutCtx::new());
        super::super::instances::set_highlighted(
            &app.instances,
            Some("opencode:review:ses_review".into()),
        );
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('.'))), &mut ctx);
        app.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                let mut render = RenderCtx::new();
                app.render(frame, area, &mut render);
                render.flush(frame);
            })
            .unwrap();
        let text = rendered_lines(&terminal, area).join("\n");
        let expected = if attached { "Goto panel" } else { "Open panel" };
        assert!(
            text.lines()
                .any(|line| line.contains(expected) && line.trim_end().ends_with("⌃;")),
            "{text}"
        );
        assert_eq!(
            text.lines()
                .any(|line| line.contains("Close session") && line.trim_end().ends_with('c')),
            attached,
            "{text}"
        );

        app.event(&TuiEvent::Key(KeyEvent::from(Key::Esc)), &mut ctx);
        assert!(!app.menu_layer().is_active());
        super::super::instances::set_highlighted(
            &app.instances,
            Some("opencode:review:ses_review".into()),
        );
        let mut shortcut_ctx = EventCtx::new(AnimationSettings::default());
        app.event(
            &TuiEvent::Key(KeyEvent {
                code: Key::Char(';'),
                modifiers: KeyModifiers::CONTROL,
            }),
            &mut shortcut_ctx,
        );
        if attached {
            assert!(shortcut_ctx.notifications().is_empty());
            assert!(app.opencode_action.is_some());
        } else {
            assert_eq!(shortcut_ctx.notifications().len(), 1);
            assert_eq!(
                shortcut_ctx.notifications()[0].title(),
                "Cannot open OpenCode"
            );
        }
        assert!(!app.view.is_active());
    }
}

#[test]
fn c_requests_closing_the_selected_opencode_session_pane() {
    tuicore::init();
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let mut app = root(AppService::for_tests());
    app.service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: vec![Session {
            id: "ses_review".into(),
            title: "Review".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Idle,
            panes: vec![pane],
            ..Default::default()
        }],
        ..Default::default()
    });
    app.update_snapshot(snapshot());
    app.layout(Rect::new(0, 0, 130, 40), &mut tuicore::LayoutCtx::new());
    super::super::instances::set_highlighted(
        &app.instances,
        Some("opencode:review:ses_review".into()),
    );
    assert!(matches!(
        app.selected().and_then(|row| row.opencode),
        Some(super::super::opencode::Target::Session {
            pane: Some(Pane { id: 7, .. }),
            ..
        })
    ));

    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('c'))), &mut ctx);

    assert!(app.opencode_action.is_some());
    assert!(ctx.notifications().is_empty());
    assert!(app.selected().is_none());
}

#[test]
fn external_client_menu_and_c_hotkey_close_its_observed_pane() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.service
        .set_opencode_snapshot_for_tests(external_observation());
    app.update_snapshot(snapshot());
    let area = Rect::new(0, 0, 130, 40);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let id = "opencode-client:external:/tmp/prototype:main:18";
    super::super::instances::set_highlighted(&app.instances, Some(id.into()));

    let mut ctx = EventCtx::new(AnimationSettings::default());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('.'))), &mut ctx);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let text = rendered_lines(&terminal, area).join("\n");
    assert!(
        text.lines()
            .any(|line| line.contains("Goto panel") && line.trim_end().ends_with("⌃;")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.contains("Close session") && line.trim_end().ends_with('c')),
        "{text}"
    );

    app.event(&TuiEvent::Key(KeyEvent::from(Key::Esc)), &mut ctx);
    super::super::instances::set_highlighted(&app.instances, Some(id.into()));
    let mut close = EventCtx::new(AnimationSettings::default());
    app.event(&TuiEvent::Key(KeyEvent::from(Key::Char('c'))), &mut close);

    assert!(app.opencode_action.is_some());
    assert!(close.notifications().is_empty());
    assert!(app.selected().is_none());
}

#[test]
fn external_client_ctrl_semicolon_navigates_from_overview_and_attached_views() {
    tuicore::init();
    for attached_only in [false, true] {
        let mut app = root(AppService::for_tests());
        app.service
            .set_opencode_snapshot_for_tests(external_observation());
        app.update_snapshot(snapshot());
        let mut ctx = EventCtx::new(AnimationSettings::default());
        if attached_only {
            app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut ctx);
        }
        app.layout(Rect::new(0, 0, 130, 40), &mut tuicore::LayoutCtx::new());
        super::super::instances::set_highlighted(
            &app.instances,
            Some("opencode-client:external:/tmp/prototype:main:18".into()),
        );

        let mut shortcut = EventCtx::new(AnimationSettings::default());
        app.event(
            &TuiEvent::Key(KeyEvent {
                code: Key::Char(';'),
                modifiers: KeyModifiers::CONTROL,
            }),
            &mut shortcut,
        );

        assert!(app.opencode_action.is_some());
        assert!(shortcut.notifications().is_empty());
        assert!(!app.view.is_active());
    }
}

#[test]
fn c_on_an_instance_confirms_closing_all_opencode_sessions() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.set_rows_for_tests(rows::from_snapshot(&snapshot()));
    super::super::instances::set_highlighted(&app.instances, Some("instance:review".into()));

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('c'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert!(matches!(
        app.intent,
        Some(super::super::Intent::CloseOpencodeSessions(ref name)) if name == "review"
    ));
    let area = Rect::new(0, 0, 130, 40);
    app.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut render = RenderCtx::new();
            app.render(frame, area, &mut render);
            render.flush(frame);
        })
        .unwrap();
    let text = rendered_lines(&terminal, area).join("\n");
    assert!(text.contains("Close all OpenCode sessions"), "{text}");
    assert!(text.contains("Ok (o) · Cancel (c)"), "{text}");

    let route = EventRoute::new(tuicore::TreePath::from_keys([tuicore::ChildKey::second()]));
    for (key, submit) in [('o', true), ('c', false)] {
        let mut action = EventCtx::new(AnimationSettings::default());
        app.dispatch_event(
            &route,
            &TuiEvent::Key(KeyEvent::from(Key::Char(key))),
            &mut action,
        );
        assert_eq!(matches!(action.messages(), [Msg::Submit]), submit);
        assert_eq!(matches!(action.messages(), [Msg::Close]), !submit);
    }
}

#[test]
fn c_on_the_sessions_group_confirms_closing_all_instance_sessions() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: vec![Session {
            id: "ses_review".into(),
            title: "Review".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Idle,
            panes: vec![Pane {
                session: "main".into(),
                id: 7,
                tab_id: 4,
                tab_name: "review".into(),
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    app.update_snapshot(snapshot());
    super::super::instances::set_highlighted(&app.instances, Some("sessions:review".into()));

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('c'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );

    assert!(matches!(
        app.intent,
        Some(super::super::Intent::CloseOpencodeSessions(ref name)) if name == "review"
    ));
    assert!(app.view.is_active());
}

#[test]
fn closing_a_pane_hides_it_across_stale_observations_and_restores_it_on_failure() {
    tuicore::init();
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let observation = Snapshot {
        sessions: vec![Session {
            id: "ses_review".into(),
            title: "Review".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Idle,
            panes: vec![pane.clone()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut app = root(AppService::for_tests());
    app.service
        .set_opencode_snapshot_for_tests(observation.clone());
    app.update_snapshot(snapshot());

    let closing = app.optimistically_close_opencode_pane("ses_review", &pane);
    super::super::instances::set_highlighted(
        &app.instances,
        Some("opencode:review:ses_review".into()),
    );
    assert!(app.selected().is_none());

    app.update_snapshot(snapshot());
    assert!(app.selected().is_none());

    let (sender, receiver) = tokio::sync::oneshot::channel();
    sender.send(Err("close failed".into())).unwrap();
    app.opencode_action = Some(super::super::opencode::PendingAction::new(
        receiver,
        "Cannot close OpenCode session",
        vec![closing],
    ));
    app.poll_opencode_action();
    super::super::instances::set_highlighted(
        &app.instances,
        Some("opencode:review:ses_review".into()),
    );
    assert!(app.selected().is_some());
}

#[test]
fn enter_opens_a_bottom_conversation_dialog_without_jumping_to_the_pane() {
    tuicore::init();
    for width in [80, 130] {
        let mut app = root(AppService::for_tests());
        app.service.set_opencode_snapshot_for_tests(Snapshot {
            sessions: vec![Session {
                id: "ses_review".into(),
                title: "Review".into(),
                directory: "/tmp/workspaces/review".into(),
                activity: Activity::Idle,
                panes: vec![Pane {
                    session: "main".into(),
                    id: 7,
                    tab_id: 4,
                    tab_name: "review".into(),
                }],
                ..Default::default()
            }],
            clients: Vec::new(),
            error: None,
        });
        app.update_snapshot(snapshot());
        let area = Rect::new(0, 0, width, 40);
        if width == 130 {
            app.handle_message(
                Msg::SetAttachedSessionsOnly(true),
                &mut EventCtx::new(AnimationSettings::default()),
            );
        }
        app.layout(area, &mut tuicore::LayoutCtx::new());
        super::super::instances::set_highlighted(
            &app.instances,
            Some("opencode:review:ses_review".into()),
        );
        let mut ctx = EventCtx::new(AnimationSettings::default());
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut ctx);
        assert!(app.details_open);
        assert!(app.view.is_active());
        assert!(app.opencode_action.is_none());
        app.view.tick(
            std::time::Duration::from_secs(1),
            AnimationSettings {
                enabled: false,
                ..Default::default()
            },
        );
        app.layout(area, &mut tuicore::LayoutCtx::new());
        let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
        terminal
            .draw(|frame| {
                let mut render = RenderCtx::new();
                app.render(frame, area, &mut render);
                render.flush(frame);
            })
            .unwrap();
        let lines = rendered_lines(&terminal, area);
        assert!(
            lines
                .iter()
                .position(|line| line.contains("Conversation"))
                .is_some_and(|index| index > 3),
            "{lines:#?}"
        );
        let text = lines.join("\n");
        assert!(text.contains("Conversation"), "{text}");
        assert!(
            text.contains("Details") && text.contains("Actions"),
            "{text}"
        );
        app.handle_message(Msg::Close, &mut ctx);
        assert!(!app.view.is_active());
    }
}
