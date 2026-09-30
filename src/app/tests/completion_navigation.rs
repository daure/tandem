use super::*;
use crate::{
    app::{
        instances::{self, Instances},
        opencode as projection,
    },
    store::opencode::{Activity, Pane, Session, Snapshot},
};
use std::time::Duration;

fn observation(activity: Activity) -> Snapshot {
    Snapshot {
        sessions: [
            ("first", "/tmp/workspaces/review", activity),
            ("neighbour", "/tmp/workspaces/review", Activity::Idle),
            ("second", "/tmp/workspaces/second", activity),
            ("external", "/work/external", activity),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (id, directory, activity))| Session {
            id: id.into(),
            title: format!("Conversation {id}"),
            directory: directory.into(),
            activity,
            updated: 10 - index as u64,
            panes: vec![Pane {
                session: "main".into(),
                id: index as u32,
                tab_id: 1,
                tab_name: "work".into(),
            }],
            ..Default::default()
        })
        .collect(),
        ..Default::default()
    }
}

fn project(sessions_only: bool, activity: Activity) -> Vec<rows::Row> {
    project_observation(sessions_only, &observation(activity))
}

fn project_observation(sessions_only: bool, observation: &Snapshot) -> Vec<rows::Row> {
    let mut inventory = snapshot();
    inventory.instances.push(Instance {
        name: "second".into(),
        workspace: "/tmp/workspaces/second".into(),
        ..inventory.instances[0].clone()
    });
    let mut rows = rows::from_snapshot(&inventory);
    if sessions_only {
        projection::attached_rows(rows, observation)
    } else {
        projection::append_rows(&mut rows, observation, false);
        rows
    }
}

fn view(sessions_only: bool, activity: Activity) -> (Instances, instances::SharedState) {
    let state = instances::state(project(sessions_only, activity));
    instances::set_attached_sessions_only(&state, sessions_only);
    let mut view = Instances::new(state.clone());
    render(&mut view);
    view.focus(None, true, &mut tuicore::FocusCtx::default());
    (view, state)
}

fn render(view: &mut Instances) -> String {
    let area = Rect::new(0, 0, 100, 12);
    view.layout(area, &mut tuicore::LayoutCtx::new());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            view.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    rendered_lines(&terminal, area).join("\n")
}

fn press(view: &mut Instances, key: KeyEvent, routed: bool) {
    let event = TuiEvent::Key(key);
    let mut ctx = EventCtx::new(AnimationSettings::default());
    if routed {
        let mut layout = LayoutEngine::new();
        layout.layout(view, Rect::new(0, 0, 100, 12));
        let route = EventRoute::new(layout.focus_targets()[0].path.clone());
        view.dispatch_event(&route, &event, &mut ctx);
    } else {
        view.event(&event, &mut ctx);
    }
}

fn selected_session(state: &instances::SharedState) -> String {
    let row = instances::selected(state).unwrap();
    let Some(projection::Target::Session { id, .. }) = row.opencode else {
        panic!("expected a conversation, selected {}", row.id);
    };
    id
}

#[test]
fn completion_navigation_expands_ancestors_and_reveals_targets_in_both_tabs() {
    init_ui();
    for sessions_only in [false, true] {
        let (mut view, state) = view(sessions_only, Activity::Busy);
        instances::replace_rows(&state, project(sessions_only, Activity::Idle));
        render(&mut view);
        press(&mut view, Key::Char('z').into(), false);
        assert!(!render(&mut view).contains("Conversation"));

        let [top, middle, bottom] = ["external", "first", "second"];
        for (key, expected) in [
            ('J', top),
            ('J', middle),
            ('J', bottom),
            ('J', bottom),
            ('K', middle),
            ('K', top),
            ('K', top),
        ] {
            press(&mut view, Key::Char(key).into(), true);
            assert_eq!(selected_session(&state), expected);
            let text = render(&mut view);
            assert!(text.contains(&format!("Conversation {expected}")), "{text}");
        }

        view.highlight_for_tests("instance:second");
        press(
            &mut view,
            KeyEvent {
                code: Key::Char('k'),
                modifiers: KeyModifiers::SHIFT,
            },
            false,
        );
        assert_eq!(selected_session(&state), "first");
        press(
            &mut view,
            KeyEvent {
                code: Key::Char('j'),
                modifiers: KeyModifiers::SHIFT,
            },
            false,
        );
        assert_eq!(selected_session(&state), "second");
    }
}

#[test]
fn session_navigation_is_inert_without_busy_sessions_or_active_markers() {
    init_ui();
    for sessions_only in [false, true] {
        for (initial, current, elapsed) in [
            (Activity::Idle, Activity::Idle, 0),
            (Activity::Busy, Activity::Idle, 20_750),
            (Activity::Unknown, Activity::Unknown, 0),
        ] {
            let (mut view, state) = view(sessions_only, initial);
            instances::replace_rows(&state, project(sessions_only, Activity::Idle));
            render(&mut view);
            instances::replace_rows(&state, project(sessions_only, current));
            view.tick(Duration::from_millis(elapsed), AnimationSettings::default());
            press(&mut view, Key::Char('z').into(), false);
            let before = render(&mut view);
            let selected = instances::selected(&state).unwrap().id;
            for key in ['J', 'K'] {
                press(&mut view, Key::Char(key).into(), true);
                assert_eq!(instances::selected(&state).unwrap().id, selected);
                assert_eq!(render(&mut view), before);
            }
        }
    }
}

#[test]
fn session_navigation_selects_busy_sessions_without_completion_markers_in_both_tabs() {
    init_ui();
    for sessions_only in [false, true] {
        let (mut view, state) = view(sessions_only, Activity::Busy);
        press(&mut view, Key::Char('z').into(), false);
        for (key, expected) in [
            ('J', "external"),
            ('J', "first"),
            ('J', "second"),
            ('J', "second"),
            ('K', "first"),
            ('K', "external"),
            ('K', "external"),
        ] {
            press(&mut view, Key::Char(key).into(), true);
            assert_eq!(selected_session(&state), expected);
            let text = render(&mut view);
            assert!(text.contains(&format!("Conversation {expected}")), "{text}");
        }
    }
}

#[test]
fn session_navigation_combines_busy_sessions_and_recent_completions_in_tree_order() {
    init_ui();
    for sessions_only in [false, true] {
        let (mut view, state) = view(sessions_only, Activity::Busy);
        let mut current = observation(Activity::Idle);
        current.sessions[2].activity = Activity::Busy;
        instances::replace_rows(&state, project_observation(sessions_only, &current));
        render(&mut view);
        press(&mut view, Key::Char('z').into(), false);
        for (key, expected) in [
            ('J', "external"),
            ('J', "first"),
            ('J', "second"),
            ('K', "first"),
            ('K', "external"),
        ] {
            press(&mut view, Key::Char(key).into(), true);
            assert_eq!(selected_session(&state), expected);
        }

        view.tick(Duration::from_millis(20_750), AnimationSettings::default());
        press(&mut view, Key::Char('J').into(), true);
        assert_eq!(selected_session(&state), "second");
        press(&mut view, Key::Char('K').into(), true);
        assert_eq!(selected_session(&state), "second");
    }
}

#[test]
fn completion_navigation_retains_the_remaining_window_across_tab_changes() {
    init_ui();
    let (mut view, state) = view(true, Activity::Busy);
    instances::replace_rows(&state, project(true, Activity::AwaitingAnswer));
    render(&mut view);
    view.tick(Duration::from_millis(10_000), AnimationSettings::default());
    for sessions_only in [false, true] {
        instances::set_attached_sessions_only(&state, sessions_only);
        instances::replace_rows(&state, project(sessions_only, Activity::AwaitingAnswer));
        render(&mut view);
        view.highlight_for_tests("instance:review");
        press(&mut view, Key::Char('J').into(), true);
        assert_eq!(selected_session(&state), "first");
    }
    view.tick(Duration::from_millis(10_749), AnimationSettings::default());
    press(&mut view, Key::Char('J').into(), true);
    assert_eq!(selected_session(&state), "second");
    view.tick(Duration::from_millis(1), AnimationSettings::default());
    for key in ['J', 'K'] {
        press(&mut view, Key::Char(key).into(), true);
        assert_eq!(selected_session(&state), "second");
    }
}

#[test]
fn completion_navigation_preserves_search_input_and_reveals_a_filtered_target() {
    init_ui();
    let (mut view, state) = view(true, Activity::Busy);
    instances::replace_rows(&state, project(true, Activity::Idle));
    render(&mut view);
    for key in ['/', 'J', 'K'] {
        press(&mut view, Key::Char(key).into(), true);
    }
    assert_eq!(view.search_query(), "JK");
    press(&mut view, Key::Enter.into(), true);
    press(&mut view, Key::Char('J').into(), true);
    assert_eq!(selected_session(&state), "external");
    assert!(view.search_query().is_empty());
    assert!(render(&mut view).contains("Conversation external"));
}
