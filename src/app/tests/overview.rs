use super::*;
use crate::app::{instances, opencode::PendingAction, operations::Deletion};
use std::time::Duration;

fn app() -> App {
    tuicore::init();
    let service = AppService::for_tests();
    service.set_opencode_snapshot_for_tests(super::attached_sessions::observation());
    let mut app = crate::app::root(service);
    app.update_snapshot(snapshot());
    app
}

fn render(app: &mut App) -> (tuicore::LayoutCtx, String) {
    let area = Rect::new(0, 0, 130, 40);
    let mut layout = tuicore::LayoutCtx::new();
    app.layout(area, &mut layout);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = RenderCtx::new();
            app.render(frame, area, &mut ctx);
            ctx.flush(frame);
        })
        .unwrap();
    (layout, rendered_lines(&terminal, area).join("\n"))
}

fn key(app: &mut App, key: Key) {
    let (layout, _) = render(app);
    let target = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::TREE_FOCUS)
        .unwrap();
    app.dispatch_focus(target, true, &mut tuicore::FocusCtx::default());
    app.dispatch_event(
        &EventRoute::new(target.path.clone()),
        &TuiEvent::Key(KeyEvent::from(key)),
        &mut EventCtx::new(AnimationSettings {
            enabled: false,
            ..Default::default()
        }),
    );
}

fn switch(app: &mut App) {
    key(app, Key::Char(']'));
    render(app);
}

fn select(app: &mut App, id: &str) {
    key(app, Key::Home);
    for _ in 0..200 {
        render(app);
        if app.selected().is_some_and(|row| row.id == id) {
            return;
        }
        key(app, Key::Down);
    }
    panic!("Could not select {id}");
}

#[test]
fn tabs_retain_independent_expansion_and_selection() {
    let mut app = app();
    assert!(render(&mut app).1.contains("Conversation busy"));
    key(&mut app, Key::Char('z'));
    assert!(!render(&mut app).1.contains("Conversation busy"));
    let sessions_selection = app.selected().unwrap().id;

    switch(&mut app);
    assert!(render(&mut app).1.contains("review"));
    key(&mut app, Key::Char('z'));
    assert!(!render(&mut app).1.contains("review"));
    let instances_selection = app.selected().unwrap().id;
    assert_ne!(sessions_selection, instances_selection);

    switch(&mut app);
    assert!(!render(&mut app).1.contains("Conversation busy"));
    assert_eq!(app.selected().unwrap().id, sessions_selection);
    key(&mut app, Key::Char('z'));
    assert!(render(&mut app).1.contains("Conversation busy"));

    switch(&mut app);
    assert!(!render(&mut app).1.contains("review"));
    assert_eq!(app.selected().unwrap().id, instances_selection);
}

#[test]
fn external_workspace_expansion_survives_switches_and_the_sessions_overview_shortcut() {
    let mut app = app();
    let mut observation = super::attached_sessions::observation();
    let mut session = observation.sessions[0].clone();
    session.directory = "/work/external".into();
    session.id = "external".into();
    session.title = "External conversation".into();
    observation.sessions.push(session);
    app.service.set_opencode_snapshot_for_tests(observation);
    app.update_snapshot(snapshot());
    select(&mut app, "opencode-workspace:/work/external");
    key(&mut app, Key::Left);
    assert!(!render(&mut app).1.contains("External conversation"));

    switch(&mut app);
    select(&mut app, "opencode-workspaces");
    key(&mut app, Key::Right);
    select(&mut app, "opencode-workspace:/work/external");
    key(&mut app, Key::Right);
    assert!(render(&mut app).1.contains("External conversation"));

    switch(&mut app);
    assert!(!render(&mut app).1.contains("External conversation"));
    switch(&mut app);
    assert!(render(&mut app).1.contains("External conversation"));
    let (layout, _) = render(&mut app);
    let tree = layout
        .focus_targets()
        .iter()
        .find(|target| target.id.as_str() == crate::app::TREE_FOCUS)
        .unwrap();
    app.dispatch_event(
        &EventRoute::new(tree.path.clone()),
        &TuiEvent::Hotkey(HotkeyEvent::Commit("shift+h".into())),
        &mut EventCtx::new(AnimationSettings::default()),
    );
    assert!(app.attached_sessions_only);
    assert!(render(&mut app).1.contains("External conversation"));
    switch(&mut app);
    assert!(render(&mut app).1.contains("External conversation"));
    assert_eq!(
        app.selected().unwrap().id,
        "opencode-workspace:/work/external"
    );
}

#[test]
fn purge_updates_the_inactive_tab_without_resetting_surviving_rows() {
    for purged in ["review", "zulu"] {
        let mut app = app();
        let mut inventory = snapshot();
        let mut other = inventory.instances[0].clone();
        other.name = "zulu".into();
        other.workspace = "/tmp/workspaces/zulu".into();
        inventory.instances.push(other);
        let mut observation = super::attached_sessions::observation();
        let mut session = observation.sessions[0].clone();
        session.id = "zulu".into();
        session.title = "Zulu conversation".into();
        session.directory = "/tmp/workspaces/zulu".into();
        observation.sessions.push(session);
        app.service.set_opencode_snapshot_for_tests(observation);
        app.update_snapshot(inventory.clone());
        select(&mut app, "instance:review");
        key(&mut app, Key::Left);
        switch(&mut app);
        select(&mut app, &format!("instance:{purged}"));
        key(&mut app, Key::Char('p'));
        assert!(matches!(&app.intent, Some(crate::app::Intent::Purge(name)) if name == purged));
        app.handle_message(Msg::Close, &mut EventCtx::new(AnimationSettings::default()));

        let operation = super::operations::operation("delete_instance", purged);
        let mut deletion = Deletion::new(operation.clone()).unwrap();
        assert!(deletion.project(&mut inventory, |_| Ok(operation), &mut Vec::new()));
        app.deletions.push(deletion);
        app.update_snapshot(inventory);
        app.pages_mut()
            .tick(Duration::ZERO, AnimationSettings::default());
        let sessions = app.pages_mut().states()[1].clone();
        assert_eq!(
            instances::selected(&sessions).unwrap().id,
            if purged == "review" {
                "instance:zulu"
            } else {
                "instance:review"
            }
        );
        switch(&mut app);
        let text = render(&mut app).1;
        assert!(!text.contains("Conversation busy"));
        assert_eq!(text.contains("Zulu conversation"), purged == "review");
    }
}

#[test]
fn asynchronous_creation_selects_only_its_originating_tab() {
    for origin_sessions in [false, true] {
        let mut app = app();
        if !origin_sessions {
            switch(&mut app);
        }
        render(&mut app);
        let (sender, reply) = tokio::sync::oneshot::channel();
        app.opencode_action = Some(PendingAction::creation(reply, app.instances.clone()));
        switch(&mut app);
        key(&mut app, Key::Char('z'));
        let selection = app.selected().unwrap().id;
        let before = render(&mut app).1;
        let mut observation = super::attached_sessions::observation();
        let pane = crate::store::opencode::Pane {
            session: "new".into(),
            id: 99,
            tab_id: 1,
            tab_name: "new".into(),
        };
        observation.clients.push(crate::store::opencode::Client {
            pane: pane.clone(),
            title: "Created client".into(),
            directory: "/tmp/workspaces/review".into(),
            ..observation.clients[0].clone()
        });
        app.service.set_opencode_snapshot_for_tests(observation);
        sender.send(Ok(pane)).unwrap();
        app.poll_opencode_action();
        app.update_snapshot(snapshot());
        app.pages_mut()
            .tick(Duration::ZERO, AnimationSettings::default());
        assert_eq!(app.selected().unwrap().id, selection);
        assert_eq!(render(&mut app).1, before);
        switch(&mut app);
        assert_eq!(app.selected().unwrap().id, "opencode-client:review:new:99");
        assert!(render(&mut app).1.contains("Created client"));
    }
}

#[test]
fn tab_round_trips_retain_each_scroll_position() {
    let mut app = app();
    let mut inventory = snapshot();
    inventory.instances = (0..30)
        .map(|index| {
            let mut instance = inventory.instances[0].clone();
            instance.name = format!("instance-{index:02}");
            instance.workspace = format!("/tmp/workspaces/{}", instance.name);
            instance
        })
        .collect();
    let mut observation = super::attached_sessions::observation();
    observation.clients.clear();
    observation.sessions = inventory
        .instances
        .iter()
        .map(|instance| {
            let mut session = observation.sessions[0].clone();
            session.id = instance.name.clone();
            session.title = format!("Conversation {}", instance.name);
            session.directory = instance.workspace.clone();
            session
        })
        .collect();
    app.service.set_opencode_snapshot_for_tests(observation);
    app.update_snapshot(inventory);
    select(&mut app, "instance:instance-15");
    let sessions = render(&mut app).1;
    switch(&mut app);
    select(&mut app, "instance:instance-05");
    let instances = render(&mut app).1;
    for _ in 0..2 {
        switch(&mut app);
        assert_eq!(render(&mut app).1, sessions);
        switch(&mut app);
        assert_eq!(render(&mut app).1, instances);
    }
}
