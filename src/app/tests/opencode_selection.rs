use super::*;
use crate::{
    app::{
        instances::{self, Instances},
        opencode as projection,
    },
    store::opencode::{Activity, Client, Pane, Session, Snapshot},
};

fn pane(id: u32) -> Pane {
    Pane {
        session: "main".into(),
        id,
        tab_id: 4,
        tab_name: "review".into(),
    }
}

fn home() -> Snapshot {
    Snapshot {
        clients: vec![Client {
            title: "OpenCode".into(),
            directory: "/tmp/workspaces/review".into(),
            server: String::new(),
            pane: pane(7),
            stale: false,
            awaiting_presence_since: None,
        }],
        ..Default::default()
    }
}

fn conversation() -> Snapshot {
    Snapshot {
        sessions: vec![Session {
            id: "ses_new".into(),
            title: "New conversation".into(),
            directory: "/tmp/workspaces/review".into(),
            activity: Activity::Busy,
            panes: vec![pane(7)],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn project(observation: &Snapshot, agents: bool) -> Vec<rows::Row> {
    if agents {
        projection::attached_rows(rows::from_snapshot(&snapshot()), observation)
    } else {
        let mut rows = rows::from_snapshot(&snapshot());
        projection::append_rows(&mut rows, observation, false);
        rows
    }
}

fn select(tree: &mut Instances, state: &instances::SharedState, id: &str) {
    tree.focus(None, true, &mut tuicore::FocusCtx::default());
    let mut ctx = EventCtx::new(AnimationSettings::default());
    for _ in 0..20 {
        if instances::selected(state).is_some_and(|row| row.id == id) {
            return;
        }
        tree.event(&TuiEvent::Key(KeyEvent::from(Key::Char('j'))), &mut ctx);
    }
    panic!("Could not select {id}");
}

#[test]
fn selection_follows_the_pane_between_home_and_its_first_conversation() {
    tuicore::init();
    for agents in [false, true] {
        let state = instances::state(project(&home(), agents));
        instances::set_attached_sessions_only(&state, agents);
        let mut tree = Instances::new(state.clone());
        tree.expand_for_tests("sessions:review");
        select(&mut tree, &state, "opencode-client:review:main:7");

        instances::replace_rows(&state, project(&conversation(), agents));
        tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
        assert_eq!(
            instances::selected(&state).unwrap().id,
            "opencode:review:ses_new"
        );

        instances::request_center_highlighted(&state);
        instances::replace_rows(&state, project(&home(), agents));
        tree.layout(Rect::new(0, 0, 120, 40), &mut tuicore::LayoutCtx::new());
        assert_eq!(
            instances::selected(&state).unwrap().id,
            "opencode-client:review:main:7"
        );
    }
}

#[test]
fn selection_follows_the_exact_pane_when_a_conversation_has_multiple_clients() {
    tuicore::init();
    let state = instances::state(project(&home(), false));
    let mut tree = Instances::new(state.clone());
    tree.expand_for_tests("sessions:review");
    select(&mut tree, &state, "opencode-client:review:main:7");

    let mut observation = conversation();
    observation.sessions[0].panes.insert(0, pane(8));
    observation.sessions[0].panes[1].tab_name = "renamed".into();
    instances::replace_rows(&state, project(&observation, false));
    tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
    assert_eq!(
        instances::selected(&state).unwrap().id,
        "opencode:review:ses_new:main:7"
    );
}

#[test]
fn a_conversation_transition_does_not_steal_selection_from_another_row() {
    tuicore::init();
    let state = instances::state(project(&home(), false));
    let mut tree = Instances::new(state.clone());
    tree.expand_for_tests("sessions:review");
    select(&mut tree, &state, "services:review");
    instances::replace_rows(&state, project(&conversation(), false));
    tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
    assert_eq!(instances::selected(&state).unwrap().id, "services:review");
}

#[test]
fn a_closed_client_does_not_follow_a_pane_with_the_same_number_in_another_zellij_session() {
    tuicore::init();
    let state = instances::state(project(&home(), false));
    let mut tree = Instances::new(state.clone());
    tree.expand_for_tests("sessions:review");
    select(&mut tree, &state, "opencode-client:review:main:7");
    let mut observation = conversation();
    observation.sessions[0].panes[0].session = "other".into();
    instances::replace_rows(&state, project(&observation, false));
    tree.tick(std::time::Duration::ZERO, AnimationSettings::default());
    assert_eq!(instances::selected(&state).unwrap().id, "services:review");
}
