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
fn completed_session_pulses_twice_over_600ms_with_selection_and_text_intact() {
    tuicore::init();
    for focused in [false, true] {
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
        instances::replace_rows(&state, rows(Activity::Idle, false));
        let (baseline, after) = render(&mut view);
        let y = after
            .iter()
            .position(|line| line.contains("Completing session"))
            .unwrap() as u16;
        let neighbour_y = after
            .iter()
            .position(|line| line.contains("Idle neighbour"))
            .unwrap() as u16;
        assert!(usize::from(y) < previous_y);
        for (step, strength) in [0.1, 0.2, 0.1, 0.0, 0.1, 0.2, 0.1, 0.0]
            .into_iter()
            .enumerate()
        {
            advance(&mut view, 75);
            if step == 1 {
                let mut refreshed = rows(Activity::Idle, false);
                for row in &mut refreshed {
                    row.metrics.age_seconds = Some(1);
                }
                instances::replace_rows(&state, refreshed);
            }
            let (actual, lines) = render(&mut view);
            assert!(lines[y as usize].contains("Completing session"));
            for line in y..y + 2 {
                for x in 0..80 {
                    let original = baseline.cell((x, line)).unwrap();
                    let cell = actual.cell((x, line)).unwrap();
                    let expected = if strength == 0.0 {
                        original.bg
                    } else {
                        let theme = tuicore::theme();
                        let bg = if original.bg == Color::Reset {
                            theme.background_bg()
                        } else {
                            original.bg
                        };
                        let bg = if matches!(bg, Color::Rgb(..)) {
                            bg
                        } else {
                            theme.dialog_bg()
                        };
                        tuicore::lerp_color(bg, theme.success_fg(), strength)
                    };
                    assert_eq!(
                        cell.bg, expected,
                        "focused={focused}, step={step}, x={x}, y={line}"
                    );
                    assert_eq!(cell.fg, original.fg);
                    assert_eq!(cell.modifier, original.modifier);
                }
            }
            for line in neighbour_y..neighbour_y + 2 {
                for x in 0..80 {
                    assert_eq!(actual.cell((x, line)), baseline.cell((x, line)));
                }
            }
        }
        assert!(
            !view
                .tick(Duration::from_millis(50), AnimationSettings::default())
                .active
        );
    }
}

#[test]
fn disabling_animations_clears_the_pulse_without_replaying_on_reenable() {
    tuicore::init();
    let state = instances::state(rows(Activity::Busy, false));
    instances::set_attached_sessions_only(&state, true);
    let mut view = Instances::new(state.clone());
    render(&mut view);
    instances::replace_rows(&state, rows(Activity::Idle, false));
    let (baseline, _) = render(&mut view);
    advance(&mut view, 150);
    assert_ne!(render(&mut view).0, baseline);
    let disabled = AnimationSettings {
        enabled: false,
        ..Default::default()
    };
    assert!(!view.tick(Duration::ZERO, disabled).active);
    assert_eq!(render(&mut view).0, baseline);
    advance(&mut view, 300);
    assert_eq!(render(&mut view).0, baseline);
}

#[test]
fn initial_stale_and_view_switch_observations_do_not_pulse() {
    tuicore::init();
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
        let (baseline, _) = render(&mut view);
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
    tuicore::init();
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
fn completion_sound_is_opt_in_and_plays_once_per_busy_to_idle_transition() {
    tuicore::init();
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
    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('N'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    assert!(app.completion_sound);
    assert!(app.toolbar_state.borrow().completion_sound);

    app.service
        .set_opencode_snapshot_for_tests(observation(Activity::Idle, false));
    app.update_snapshot(snapshot());
    app.update_snapshot(snapshot());
    assert_eq!(app.service.completion_sound_count_for_tests(), 1);

    app.event(
        &TuiEvent::Key(KeyEvent::from(Key::Char('N'))),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    app.service
        .set_opencode_snapshot_for_tests(observation(Activity::Busy, false));
    app.update_snapshot(snapshot());
    app.service
        .set_opencode_snapshot_for_tests(observation(Activity::Idle, false));
    app.update_snapshot(snapshot());
    assert!(!app.completion_sound);
    assert_eq!(app.service.completion_sound_count_for_tests(), 1);
}
