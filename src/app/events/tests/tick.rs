use super::*;
use crate::store::{environments::Instance, opencode::Activity};

pub(in crate::app::events) fn events(
    instance_loading: Option<bool>,
    conversation: Option<Activity>,
) -> Events {
    crate::app::tests::init_ui();
    let mut context = super::super::acceptances::Context::default();
    let mut record = Record {
        sequence: 1,
        provider: "sample".into(),
        received_at: "2026-10-06T14:32:00Z".into(),
        event: crate::environments::events::tests::event("spinner"),
        attempts: vec![],
        acceptances: vec![],
    };
    if let Some(loading) = instance_loading {
        context.inventory.instances.push(Instance {
            name: "review".into(),
            template: "blank".into(),
            workspace: "/tmp/opencode/events-spinner-review".into(),
            pending: loading,
            workspace_only: true,
            ..Default::default()
        });
        record.acceptances.push(
            serde_json::from_value(serde_json::json!({
                "id": 1, "event_sequence": 1, "event_summary": "spinner",
                "attempt_id": 1, "rule_name": "matching", "rule_revision": 1,
                "accepted_at": "2026-10-06T14:32:00Z", "instance": "review",
                "status": "launched", "operation_id": "operation-1",
                "rule": {
                    "definition": {
                        "name": "matching", "script": "fn matches(event) { true }",
                        "template": "blank", "model": "openai/test", "initial_prompt": "Inspect"
                    },
                    "revision": 1, "zellij_session": "main"
                }
            }))
            .unwrap(),
        );
    }
    if let Some(activity) = conversation {
        context
            .opencode
            .sessions
            .push(crate::store::opencode::Session {
                id: "ses_spinner".into(),
                title: "Spinner conversation".into(),
                directory: "/tmp/opencode/events-spinner-review".into(),
                activity,
                last_question: Some("Inspect loading".into()),
                ..Default::default()
            });
    }
    let mut events = Events::new(
        Rc::new(RefCell::new(Snapshot {
            records: vec![record],
            total: 1,
            ..Default::default()
        })),
        Default::default(),
        Default::default(),
        Default::default(),
        Rc::new(RefCell::new(super::super::toolbar::State {
            show_saved: true,
            ..Default::default()
        })),
        Default::default(),
        Rc::new(RefCell::new(context)),
    );
    events.tick(Duration::ZERO, disabled());
    events.view.second_mut().expand(&"acceptance:1".into());
    render(&mut events);
    events.tick(Duration::from_secs(1), disabled());
    events
}

fn disabled() -> AnimationSettings {
    AnimationSettings {
        enabled: false,
        ..Default::default()
    }
}

pub(in crate::app::events) fn render(events: &mut Events) -> String {
    let area = Rect::new(0, 0, 130, 30);
    events.layout(area, &mut LayoutCtx::default());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(130, 30)).unwrap();
    terminal
        .draw(|frame| events.render(frame, area, &mut RenderCtx::new()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn settled_events_keep_relative_time_updates_without_active_animation() {
    for (instance, conversation) in [
        (None, None),
        (Some(false), None),
        (Some(false), Some(Activity::Idle)),
    ] {
        let mut events = events(instance, conversation);
        assert!(!events.projected.is_empty());
        let tick = events.tick(Duration::ZERO, AnimationSettings::default());
        assert!(!tick.active, "idle feed requested animation: {tick:?}");
        assert!(!tick.changed, "settled feed requested redraw: {tick:?}");
        assert!(
            tick.next_tick.is_some(),
            "relative time updates must remain scheduled"
        );
    }
}

#[test]
fn loading_instances_and_busy_conversations_animate_only_when_enabled() {
    for (instance, conversation) in [(Some(true), None), (Some(false), Some(Activity::Busy))] {
        let mut events = events(instance, conversation);
        assert!(render(&mut events).contains("⠋"));
        let tick = events.tick(Duration::from_millis(80), AnimationSettings::default());
        assert!(
            !tick.active
                && tick.changed
                && tick
                    .next_tick
                    .is_some_and(|wait| wait <= Duration::from_millis(80)),
            "loading feed did not animate: {tick:?}"
        );
        assert!(render(&mut events).contains("⠙"));
        let tick = events.tick(Duration::from_millis(80), disabled());
        assert!(
            !tick.active && !tick.changed,
            "disabled feed animated: {tick:?}"
        );
        assert!(render(&mut events).contains("⠙"));
    }
}

#[test]
fn busy_events_reuse_the_projection_until_inputs_change() {
    let mut events = events(Some(false), Some(Activity::Busy));
    let first = events.projections;
    for _ in 0..10 {
        events.tick(Duration::from_millis(80), disabled());
    }
    assert_eq!(events.projections, first);
    events.context.borrow_mut().opencode.sessions[0].title = "Updated conversation".into();
    events.dirty.set(true);
    events.tick(Duration::ZERO, disabled());
    assert_eq!(events.projections, first + 1);
    assert!(render(&mut events).contains("Updated conversation"));

    events.toolbar.borrow_mut().show_saved = false;
    events.tick(Duration::ZERO, disabled());
    assert_eq!(events.projections, first + 2);
}

#[test]
fn clean_events_process_focus_deletion_and_filter_requests() {
    let mut events = events(Some(false), None);
    let mut pinned = events.shared.borrow().records[0].clone();
    pinned.sequence = 20;
    events.requested.borrow_mut().record = Some(pinned);
    events.tick(Duration::ZERO, disabled());
    assert_eq!(
        events.view.second().highlighted_id().as_deref(),
        Some("event:20")
    );
    assert_eq!(events.pinned.as_ref().map(|row| row.sequence), Some(20));

    events.requested.borrow_mut().deletion = Some(Deletion::Event(20));
    events.tick(Duration::ZERO, disabled());
    assert!(events.pinned.is_none());
    assert!(!events.projected.iter().any(|row| row.id() == "event:20"));

    *events.filter.borrow_mut() = Some(vec![StreamKey::new("other".into(), "other".into())]);
    events.tick(Duration::ZERO, disabled());
    assert!(events.projected.is_empty());
    *events.filter.borrow_mut() = Some(Vec::new());
    events.requested.borrow_mut().reset_filters = true;
    events.tick(Duration::ZERO, disabled());
    assert!(!events.projected.is_empty());
}

#[test]
fn large_busy_event_feeds_reuse_shared_targets_and_settled_projections() {
    let mut events = events(Some(false), Some(Activity::Busy));
    {
        let mut context = events.context.borrow_mut();
        let instance = context.inventory.instances[0].clone();
        let session = context.opencode.sessions[0].clone();
        context.inventory.instances = (0..20)
            .map(|index| {
                let mut instance = instance.clone();
                instance.name = format!("review-{index}");
                instance.workspace = format!("/tmp/opencode/review-{index}");
                instance
            })
            .collect();
        context.opencode.sessions = (0..20)
            .flat_map(|instance| {
                let session = session.clone();
                (0..10).map(move |index| {
                    let mut session = session.clone();
                    session.id = format!("ses_{instance}_{index}");
                    session.directory = format!("/tmp/opencode/review-{instance}");
                    session
                })
            })
            .collect();
    }
    {
        let mut shared = events.shared.borrow_mut();
        let acceptance = shared.records[0].acceptances[0].clone();
        shared.records[0].acceptances = (0..50)
            .map(|index| {
                let mut acceptance = acceptance.clone();
                acceptance.id = index + 1;
                acceptance.instance = format!("review-{}", index % 20);
                acceptance
            })
            .collect();
    }
    events.dirty.set(true);
    events.tick(Duration::ZERO, disabled());
    assert_eq!(events.projected.len(), 551);
    let Entry::Acceptance(target) = &events.projected[1] else {
        panic!("acceptance row");
    };
    let Entry::Conversation { target: child, .. } = &events.projected[2] else {
        panic!("conversation row");
    };
    assert!(Rc::ptr_eq(target, child));
    let projections = events.projections;
    for _ in 0..10 {
        events.tick(Duration::from_millis(80), disabled());
    }
    assert_eq!(events.projections, projections);
}
