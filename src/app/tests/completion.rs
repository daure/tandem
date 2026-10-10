use super::*;
use crate::{
    app::instances::{self, Instances},
    store::opencode::{Activity, Pane, Session, Snapshot},
};
use ratatui::{buffer::Buffer, style::Color};
use std::time::Duration;

fn observation(activity: Activity, stale: bool) -> Snapshot {
    Snapshot {
        sessions: [
            ("completing", "Completing session", activity, 20),
            ("neighbour", "Idle neighbour", Activity::Idle, 10),
        ]
        .into_iter()
        .map(|(id, title, activity, updated)| Session {
            id: id.into(),
            title: title.into(),
            directory: "/work".into(),
            activity,
            updated,
            stale: id == "completing" && stale,
            last_question: Some("Second line".into()),
            question_observed: true,
            panes: vec![Pane {
                session: "main".into(),
                id: updated as u32,
                tab_id: 1,
                tab_name: "work".into(),
            }],
            ..Default::default()
        })
        .collect(),
        ..Default::default()
    }
}

fn rows(activity: Activity, stale: bool) -> Vec<rows::Row> {
    super::super::opencode::attached_rows(Vec::new(), &observation(activity, stale))
}

fn render(view: &mut Instances) -> (Buffer, Vec<String>) {
    let area = Rect::new(0, 0, 80, 12);
    view.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            view.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    (
        terminal.backend().buffer().clone(),
        rendered_lines(&terminal, area),
    )
}

fn advance(view: &mut Instances, milliseconds: u64) {
    for _ in 0..milliseconds / 25 {
        view.tick(Duration::from_millis(25), AnimationSettings::default());
    }
}

#[test]
fn completion_gutter_pulses_twice_then_rises_and_fades_over_20_seconds() {
    init_ui();
    for (focused, activity) in [false, true].into_iter().flat_map(|focused| {
        [Activity::Idle, Activity::AwaitingAnswer].map(|activity| (focused, activity))
    }) {
        let state = instances::state(rows(Activity::Busy, false));
        instances::set_attached_sessions_only(&state, true);
        let mut view = Instances::new(state.clone());
        render(&mut view);
        if focused {
            view.focus(None, true, &mut tuicore::FocusCtx::default());
            view.event(
                &TuiEvent::Key(KeyEvent::from(Key::End)),
                &mut EventCtx::new(AnimationSettings::default()),
            );
        }
        let (_, before) = render(&mut view);
        let previous_y = before
            .iter()
            .position(|line| line.contains("Completing session"))
            .unwrap();
        let completed = rows(activity, false);
        if activity == Activity::AwaitingAnswer {
            let row = completed
                .iter()
                .find(|row| row.id.ends_with(":completing"))
                .unwrap();
            assert_eq!(row.icon, "󱚟");
            assert_eq!(row.tone, rows::Tone::Success);
            assert_eq!(row.secondary_icon, "");
            assert_eq!(row.secondary_tone, rows::Tone::Success);
            assert!(!row.secondary_loading);
            assert!(row.activity_timer.is_none());
        }
        instances::replace_rows(&state, completed);
        let (initial, after) = render(&mut view);
        let y = after
            .iter()
            .position(|line| line.contains("Completing session"))
            .unwrap() as u16;
        let neighbour_y = after
            .iter()
            .position(|line| line.contains("Idle neighbour"))
            .unwrap() as u16;
        assert_eq!(usize::from(y), previous_y);
        let mut elapsed = 0;
        for (milliseconds, strength, marker_strength) in [
            (0, 0.0, 0.0),
            (75, 0.1, 0.5),
            (150, 0.2, 1.0),
            (225, 0.1, 0.5),
            (300, 0.0, 0.0),
            (375, 0.1, 0.5),
            (450, 0.2, 1.0),
            (525, 0.1, 0.5),
            (600, 0.0, 0.0),
            (675, 0.0, 0.5),
            (750, 0.0, 1.0),
            (5_750, 0.0, 0.75),
            (10_750, 0.0, 0.5),
            (15_750, 0.0, 0.25),
            (20_725, 0.0, 0.00125),
            (20_750, 0.0, 0.0),
        ] {
            advance(&mut view, milliseconds - elapsed);
            elapsed = milliseconds;
            if milliseconds == 5_750 {
                let mut refreshed = rows(activity, false);
                for row in &mut refreshed {
                    row.metrics.age_seconds = Some(1);
                }
                instances::replace_rows(&state, refreshed);
            }
            let (actual, lines) = render(&mut view);
            assert!(lines[y as usize].contains("Completing session"));
            for line in y..y + 2 {
                for x in 0..80 {
                    let original = initial.cell((x, line)).unwrap();
                    let cell = actual.cell((x, line)).unwrap();
                    let theme = tuicore::theme();
                    let background = |color| {
                        let color = if color == Color::Reset {
                            theme.background_bg()
                        } else {
                            color
                        };
                        if matches!(color, Color::Rgb(..)) {
                            color
                        } else {
                            theme.dialog_bg()
                        }
                    };
                    let expected_bg = if strength == 0.0 {
                        original.bg
                    } else {
                        tuicore::lerp_color(background(original.bg), theme.success_fg(), strength)
                    };
                    assert_eq!(cell.bg, expected_bg);
                    assert_eq!(cell.modifier, original.modifier);
                    if x == 0 && milliseconds < 20_750 {
                        assert_eq!(cell.symbol(), "┃");
                        assert_eq!(
                            cell.fg,
                            tuicore::lerp_color(
                                background(expected_bg),
                                theme.success_fg(),
                                marker_strength,
                            ),
                            "focused={focused}, elapsed={milliseconds}, y={line}",
                        );
                    } else if x == 0 {
                        assert_eq!(cell.symbol(), " ");
                    } else {
                        assert_eq!(cell.fg, original.fg);
                        assert_eq!(cell.symbol(), original.symbol());
                    }
                }
            }
            for line in neighbour_y..neighbour_y + 2 {
                for x in 0..80 {
                    assert_eq!(actual.cell((x, line)), initial.cell((x, line)));
                }
            }
        }
        let selected = instances::selected(&state).unwrap();
        let baseline_state = instances::state(rows(activity, false));
        instances::set_attached_sessions_only(&baseline_state, true);
        let mut baseline = Instances::new(baseline_state);
        baseline.highlight_for_tests(&selected.id);
        if focused {
            baseline.focus(None, true, &mut tuicore::FocusCtx::default());
        }
        assert_eq!(render(&mut view).1, render(&mut baseline).1);
        assert!(
            !view
                .tick(Duration::from_millis(50), AnimationSettings::default())
                .active
        );
    }
}

#[test]
fn configured_duration_controls_the_final_gutter_fade() {
    init_ui();
    let state = instances::state(rows(Activity::Busy, false));
    instances::set_attached_sessions_only(&state, true);
    instances::set_completion_fade(&state, 2);
    let mut view = Instances::new(state.clone());
    render(&mut view);
    instances::replace_rows(&state, rows(Activity::Idle, false));
    render(&mut view);
    advance(&mut view, 750);
    let (peak, lines) = render(&mut view);
    let y = lines.iter().position(|line| line.contains('┃')).unwrap() as u16;
    assert_eq!(peak.cell((0, y)).unwrap().fg, tuicore::theme().success_fg());
    advance(&mut view, 1_000);
    let (halfway, _) = render(&mut view);
    assert_eq!(halfway.cell((0, y)).unwrap().symbol(), "┃");
    assert_ne!(
        halfway.cell((0, y)).unwrap().fg,
        peak.cell((0, y)).unwrap().fg
    );
    advance(&mut view, 1_000);
    assert!(render(&mut view).1.iter().all(|line| !line.contains('┃')));
    assert!(
        !view
            .tick(Duration::ZERO, AnimationSettings::default())
            .active
    );
}

fn ping_rows(activity: Activity, revision: u64) -> Vec<rows::Row> {
    let mut rows = rows(activity, false);
    for row in &mut rows {
        if row.id.ends_with(":completing") {
            row.ping_revision = Some(revision);
        }
    }
    rows
}

#[test]
fn instance_ping_fades_keep_primary_color_through_new_work_and_completion() {
    init_ui();
    for activity in [Activity::Busy, Activity::Idle, Activity::AwaitingAnswer] {
        let state = instances::state(ping_rows(activity, 1));
        instances::set_attached_sessions_only(&state, true);
        let mut view = Instances::new(state.clone());
        let (baseline, lines) = render(&mut view);
        assert!(lines.iter().all(|line| !line.contains('┃')));
        let y = lines
            .iter()
            .position(|line| line.contains("Completing session"))
            .unwrap() as u16;
        let neighbour_y = lines
            .iter()
            .position(|line| line.contains("Idle neighbour"))
            .unwrap() as u16;
        instances::replace_rows(&state, ping_rows(activity, 2));
        render(&mut view);
        advance(&mut view, 150);
        let (pulse, _) = render(&mut view);
        let theme = tuicore::theme();
        for line in y..y + 2 {
            let bg = baseline.cell((1, line)).unwrap().bg;
            let bg = if matches!(bg, Color::Rgb(..)) {
                bg
            } else {
                theme.dialog_bg()
            };
            assert_eq!(
                pulse.cell((1, line)).unwrap().bg,
                tuicore::lerp_color(bg, theme.accent_fg(), 0.2)
            );
            assert_eq!(pulse.cell((0, line)).unwrap().fg, theme.accent_fg());
        }
        for line in neighbour_y..neighbour_y + 2 {
            for x in 0..80 {
                assert_eq!(pulse.cell((x, line)), baseline.cell((x, line)));
            }
        }
        instances::replace_rows(&state, ping_rows(Activity::Idle, 2));
        render(&mut view);
        advance(&mut view, 600);
        let (peak, _) = render(&mut view);
        assert_eq!(peak.cell((0, y)).unwrap().fg, theme.accent_fg());
        advance(&mut view, 5_000);
        let (fading, _) = render(&mut view);
        let expected =
            tuicore::lerp_color(fading.cell((0, y)).unwrap().bg, theme.accent_fg(), 0.75);
        assert_eq!(fading.cell((0, y)).unwrap().fg, expected);
        for activity in [
            Activity::Busy,
            Activity::AwaitingAnswer,
            Activity::Busy,
            Activity::Idle,
        ] {
            instances::replace_rows(&state, ping_rows(activity, 2));
            let (continued, _) = render(&mut view);
            assert_eq!(continued.cell((0, y)).unwrap().fg, expected);
            assert_eq!(
                continued.cell((1, y)).unwrap().bg,
                fading.cell((1, y)).unwrap().bg
            );
        }
        advance(&mut view, 150);
        let (continued, _) = render(&mut view);
        assert_eq!(
            continued.cell((0, y)).unwrap().fg,
            tuicore::lerp_color(
                continued.cell((0, y)).unwrap().bg,
                theme.accent_fg(),
                1.0 - 5.15 / 20.0,
            )
        );
        advance(&mut view, 14_850);
        assert!(render(&mut view).1.iter().all(|line| !line.contains('┃')));
        instances::replace_rows(&state, ping_rows(Activity::Busy, 2));
        render(&mut view);
        instances::replace_rows(&state, ping_rows(Activity::Idle, 2));
        render(&mut view);
        advance(&mut view, 750);
        assert_eq!(
            render(&mut view).0.cell((0, y)).unwrap().fg,
            theme.success_fg()
        );
        instances::replace_rows(&state, ping_rows(Activity::Idle, 3));
        render(&mut view);
        advance(&mut view, 750);
        assert_eq!(
            render(&mut view).0.cell((0, y)).unwrap().fg,
            theme.accent_fg()
        );
        advance(&mut view, 20_000);
        assert!(render(&mut view).1.iter().all(|line| !line.contains('┃')));
    }
}

#[test]
fn ping_signals_are_projected_only_into_the_owning_instances_session_rows() {
    init_ui();
    let mut environment = snapshot();
    environment.instances[0].workspace = "/work".into();
    for instance in ["other", "review"] {
        environment.session_pings = vec![crate::store::environments::SessionPing {
            instance: instance.into(),
            session_id: "completing".into(),
            revision: 10,
        }];
        let projected = super::super::opencode::project_rows(
            &environment,
            &[],
            &observation(Activity::Idle, false),
            false,
            false,
            true,
        );
        assert!(projected.iter().any(|row| row.id.ends_with(":completing")));
        for row in projected {
            assert_eq!(
                row.ping_revision,
                (instance == "review" && row.id.ends_with(":completing")).then_some(10)
            );
        }
    }
}

#[test]
fn completion_sounds_play_while_the_sessions_tree_keeps_the_original_ping_fade() {
    init_ui();
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(observation(Activity::Busy, false));
    let mut app = root(service);
    app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut EventCtx::default());
    app.handle_message(Msg::SetCompletionSound(true), &mut EventCtx::default());
    let mut inventory = snapshot();
    inventory.instances[0].workspace = "/work".into();
    app.update_snapshot(inventory.clone());
    let mut view = Instances::new(app.instances.clone());
    let (_, lines) = render(&mut view);
    let y = lines
        .iter()
        .position(|line| line.contains("Completing session"))
        .unwrap() as u16;
    inventory
        .session_pings
        .push(crate::store::environments::SessionPing {
            instance: "review".into(),
            session_id: "completing".into(),
            revision: 1,
        });
    app.update_snapshot(inventory.clone());
    render(&mut view);
    advance(&mut view, 750);
    assert_eq!(
        render(&mut view).0.cell((0, y)).unwrap().fg,
        tuicore::theme().accent_fg()
    );
    advance(&mut view, 5_000);
    let (fading, _) = render(&mut view);
    for (activity, sounds) in [
        (Activity::Idle, 1),
        (Activity::Busy, 1),
        (Activity::AwaitingAnswer, 2),
    ] {
        app.service
            .set_opencode_snapshot_for_tests(observation(activity, false));
        app.update_snapshot(inventory.clone());
        let (continued, _) = render(&mut view);
        assert_eq!(continued.cell((0, y)), fading.cell((0, y)));
        assert_eq!(
            continued.cell((1, y)).unwrap().bg,
            fading.cell((1, y)).unwrap().bg
        );
        assert_eq!(app.service.completion_sound_count_for_tests(), sounds);
    }
    advance(&mut view, 15_000);
    assert!(render(&mut view).1.iter().all(|line| !line.contains('┃')));
}

#[test]
fn disabling_animations_clears_the_marker_without_replaying_on_reenable() {
    init_ui();
    let state = instances::state(rows(Activity::Busy, false));
    instances::set_attached_sessions_only(&state, true);
    let mut view = Instances::new(state.clone());
    render(&mut view);
    instances::replace_rows(&state, rows(Activity::Idle, false));
    assert!(render(&mut view).1.iter().any(|line| line.starts_with('┃')));
    let disabled = AnimationSettings {
        enabled: false,
        ..Default::default()
    };
    assert!(!view.tick(Duration::ZERO, disabled).active);
    let (baseline, lines) = render(&mut view);
    assert!(lines.iter().all(|line| !line.contains('┃')));
    advance(&mut view, 300);
    assert_eq!(render(&mut view).0, baseline);
}

#[test]
fn completion_markers_require_a_fresh_busy_transition_in_the_same_view() {
    init_ui();
    for (previous, current, switch) in [
        (Vec::new(), rows(Activity::Idle, false), false),
        (
            rows(Activity::Idle, false),
            rows(Activity::Idle, false),
            false,
        ),
        (
            rows(Activity::Busy, true),
            rows(Activity::Idle, false),
            false,
        ),
        (
            rows(Activity::Busy, false),
            rows(Activity::Idle, true),
            false,
        ),
        (
            rows(Activity::Unknown, false),
            rows(Activity::Idle, false),
            false,
        ),
        (
            rows(Activity::Busy, false),
            rows(Activity::Idle, false),
            true,
        ),
    ] {
        let state = instances::state(previous);
        instances::set_attached_sessions_only(&state, true);
        let mut view = Instances::new(state.clone());
        render(&mut view);
        if switch {
            instances::set_attached_sessions_only(&state, false);
        }
        instances::replace_rows(&state, current);
        let (baseline, lines) = render(&mut view);
        assert!(lines.iter().all(|line| !line.contains('┃')));
        advance(&mut view, 300);
        assert_eq!(render(&mut view).0, baseline);
        assert!(
            !view
                .tick(Duration::ZERO, AnimationSettings::default())
                .active
        );
    }
}

#[test]
fn a_session_reappearing_after_filtering_does_not_replay_completion() {
    init_ui();
    let state = instances::state(rows(Activity::Busy, false));
    instances::set_attached_sessions_only(&state, true);
    let mut view = Instances::new(state.clone());
    render(&mut view);
    let mut filtered = rows(Activity::Busy, false);
    filtered.retain(|row| !row.id.ends_with(":completing"));
    instances::replace_rows(&state, filtered);
    render(&mut view);
    instances::replace_rows(&state, rows(Activity::Idle, false));
    let (baseline, _) = render(&mut view);
    advance(&mut view, 300);
    assert_eq!(render(&mut view).0, baseline);
    assert!(
        !view
            .tick(Duration::ZERO, AnimationSettings::default())
            .active
    );
}

#[test]
fn completion_markers_clear_on_new_work_and_expire_after_elapsed_time() {
    init_ui();
    let state = instances::state(rows(Activity::Busy, false));
    instances::set_attached_sessions_only(&state, true);
    let mut view = Instances::new(state.clone());
    render(&mut view);
    instances::replace_rows(&state, rows(Activity::Idle, false));
    assert!(render(&mut view).1.iter().any(|line| line.contains('┃')));
    advance(&mut view, 5_000);

    instances::replace_rows(&state, rows(Activity::Busy, false));
    assert!(render(&mut view).1.iter().all(|line| !line.contains('┃')));
    instances::replace_rows(&state, rows(Activity::Idle, false));
    render(&mut view);
    advance(&mut view, 750);
    let (buffer, lines) = render(&mut view);
    let y = lines.iter().position(|line| line.contains('┃')).unwrap() as u16;
    assert_eq!(
        buffer.cell((0, y)).unwrap().fg,
        tuicore::theme().success_fg()
    );
    assert!(
        !view
            .tick(Duration::from_secs(20), AnimationSettings::default())
            .active
    );
    assert!(render(&mut view).1.iter().all(|line| !line.contains('┃')));
}

#[test]
fn completion_sound_is_opt_in_and_plays_once_per_busy_to_idle_transition() {
    init_ui();
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(observation(Activity::Busy, false));
    let mut app = root(service);

    app.service
        .set_opencode_snapshot_for_tests(observation(Activity::Idle, false));
    app.update_snapshot(snapshot());
    assert_eq!(app.service.completion_sound_count_for_tests(), 0);

    app.service
        .set_opencode_snapshot_for_tests(observation(Activity::Busy, false));
    app.update_snapshot(snapshot());
    app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut EventCtx::default());
    app.handle_message(Msg::SetCompletionSound(true), &mut EventCtx::default());
    assert!(app.completion_sound);

    app.service
        .set_opencode_snapshot_for_tests(observation(Activity::Idle, false));
    app.update_snapshot(snapshot());
    app.update_snapshot(snapshot());
    assert_eq!(app.service.completion_sound_count_for_tests(), 1);

    app.handle_message(Msg::SetCompletionSound(false), &mut EventCtx::default());
    app.service
        .set_opencode_snapshot_for_tests(observation(Activity::Busy, false));
    app.update_snapshot(snapshot());
    app.service
        .set_opencode_snapshot_for_tests(observation(Activity::Idle, false));
    app.update_snapshot(snapshot());
    assert!(!app.completion_sound);
    assert_eq!(app.service.completion_sound_count_for_tests(), 1);
}

#[test]
fn question_waits_notify_once_and_resume_normal_completion_feedback() {
    init_ui();
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(observation(Activity::Busy, false));
    let mut app = root(service);
    app.handle_message(Msg::SetAttachedSessionsOnly(true), &mut EventCtx::default());
    app.handle_message(Msg::SetCompletionSound(true), &mut EventCtx::default());
    assert!(app.completion_sound);
    for (activity, expected_sounds) in [
        (Activity::AwaitingAnswer, 1),
        (Activity::AwaitingAnswer, 1),
        (Activity::Idle, 1),
        (Activity::Busy, 1),
        (Activity::AwaitingAnswer, 2),
        (Activity::Busy, 2),
        (Activity::Idle, 3),
    ] {
        app.service
            .set_opencode_snapshot_for_tests(observation(activity, false));
        app.update_snapshot(snapshot());
        assert_eq!(
            app.service.completion_sound_count_for_tests(),
            expected_sounds
        );
    }
}
