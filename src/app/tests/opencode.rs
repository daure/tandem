use super::super::rows::Tone;
use super::*;
use crate::store::opencode::{Activity, Client, Pane, Session, Snapshot};

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
    assert!(
        rows.iter()
            .find(|row| row.instance.is_some())
            .unwrap()
            .text("*", None)
            .to_string()
            .contains("OpenCode: 3 live · 2 attached · 2 busy")
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
        assert!(text.contains(&format!("Quick ping · {second}s")), "{text}");
        assert!(text.contains(&format!("Second ping · {second}s")), "{text}");
        tree.tick(Duration::from_millis(420), AnimationSettings::default());
        let text = render(&mut tree, &mut terminal);
        assert!(text.contains(&format!("Quick ping · {second}s")), "{text}");
        assert!(text.contains(&format!("Second ping · {second}s")), "{text}");
        observation.sessions[0].activity_elapsed_milliseconds = Some(second * 1_000 + 740);
        observation.sessions[1].activity_elapsed_milliseconds = Some(second * 1_000 + 1_000);
        instances::replace_rows(&state, make_rows(&observation));
        let text = render(&mut tree, &mut terminal);
        assert!(text.contains(&format!("Quick ping · {second}s")), "{text}");
        assert!(text.contains(&format!("Second ping · {second}s")), "{text}");
        tree.tick(Duration::from_millis(580), AnimationSettings::default());
    }
    observation.sessions[0].activity = Activity::Idle;
    observation.sessions[0].activity_started_at_milliseconds = None;
    observation.sessions[0].activity_elapsed_milliseconds = Some(5_000);
    instances::replace_rows(&state, make_rows(&observation));
    tree.tick(Duration::from_secs(10), AnimationSettings::default());
    let text = render(&mut tree, &mut terminal);
    assert!(text.contains("Quick ping · 5s"), "{text}");
    assert!(text.contains(" Sleep for 30 seconds"), "{text}");
}

#[test]
fn attached_client_without_a_conversation_appears_under_its_instance() {
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
    assert!(
        instance
            .status_detail
            .as_deref()
            .unwrap()
            .contains("1 live · 1 attached · 0 busy")
    );
    let client = rows
        .iter()
        .find(|row| row.id == "opencode-client:review:main:12")
        .unwrap();
    assert_eq!(client.parent.as_deref(), Some("instance:review"));
    assert_eq!(client.label, "OpenCode\nNew session");
    assert!(client.status.is_none());
    assert_eq!(client.tone, super::super::rows::Tone::Success);
    assert_eq!(client.secondary_icon, "");
    assert_eq!(client.secondary_tone, super::super::rows::Tone::Muted);
    let text = client.text("", Some(80));
    assert!(
        text.lines[1]
            .spans
            .iter()
            .all(|span| span.style.fg == Some(tuicore::theme().muted_fg()))
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
fn instance_children_put_opencode_sessions_before_setup_and_services() {
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
        [
            "opencode:review:ses_review",
            "setup:review",
            "services:review"
        ]
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
    assert!(
        rows[1]
            .status_detail
            .as_ref()
            .unwrap()
            .contains("1 live · 1 attached · 0 busy")
    );
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
        assert_eq!(shortcut_ctx.notifications().len(), 1);
        assert_eq!(
            shortcut_ctx.notifications()[0].title(),
            "Cannot open OpenCode"
        );
        assert!(!app.view.first().is_active());
    }
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
        app.layout(area, &mut tuicore::LayoutCtx::new());
        super::super::instances::set_highlighted(
            &app.instances,
            Some("opencode:review:ses_review".into()),
        );
        let mut ctx = EventCtx::new(AnimationSettings::default());
        app.event(&TuiEvent::Key(KeyEvent::from(Key::Enter)), &mut ctx);
        assert!(app.details_open);
        assert!(app.view.first().is_active());
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
        assert!(!app.view.first().is_active());
    }
}
