use super::*;

#[test]
fn event_relative_times_keep_their_state_through_refresh_and_schedule_live_updates() {
    crate::app::tests::init_ui();
    let row = Record {
        sequence: 1,
        provider: "github".into(),
        received_at: "2026-10-06T14:32:00Z".into(),
        event: crate::environments::events::tests::event("release"),
        attempts: vec![],
        acceptances: vec![],
    };
    let mut times = RelativeTimes::default();
    times.sync(std::slice::from_ref(&row));
    let tick = times.tick(Duration::ZERO, AnimationSettings::default());
    assert!(tick.next_tick.is_some());

    let target = chrono::DateTime::parse_from_rfc3339(&row.received_at).unwrap();
    let reference = time::OffsetDateTime::from_unix_timestamp(target.timestamp() + 30).unwrap();
    times.dates.get_mut(&1).unwrap().set_reference(reference);
    assert_eq!(times.text(1), Some("A moment ago"));
    times.sync(std::slice::from_ref(&row));
    assert_eq!(times.text(1), Some("A moment ago"));

    times
        .dates
        .get_mut(&1)
        .unwrap()
        .set_reference(reference + time::Duration::seconds(60));
    assert_eq!(times.text(1), Some("1 minute ago"));
    times.sync(&[]);
    assert!(times.dates.is_empty());
    assert_eq!(
        times.tick(Duration::ZERO, AnimationSettings::default()),
        TickResult::IDLE
    );
}

#[test]
fn acceptance_relative_times_render_without_rebuilding_the_event_tree() {
    let mut events = super::super::tick_tests::events(Some(false), None);
    let projections = events.projections;
    let accepted_at = events.shared.borrow().records[0].acceptances[0]
        .accepted_at
        .clone();
    let target = chrono::DateTime::parse_from_rfc3339(&accepted_at).unwrap();
    let reference = time::OffsetDateTime::from_unix_timestamp(target.timestamp() + 30).unwrap();
    assert!(
        events
            .relative_times
            .borrow_mut()
            .tick(Duration::ZERO, AnimationSettings::default())
            .next_tick
            .is_some()
    );
    events
        .relative_times
        .borrow_mut()
        .acceptance_dates
        .get_mut(&1)
        .unwrap()
        .set_reference(reference);
    assert!(super::super::tick_tests::render(&mut events).contains("A moment ago"));
    events
        .relative_times
        .borrow_mut()
        .acceptance_dates
        .get_mut(&1)
        .unwrap()
        .set_reference(reference + time::Duration::seconds(60));
    assert!(super::super::tick_tests::render(&mut events).contains("1 minute ago"));
    assert_eq!(events.projections, projections);
}
