use std::time::{Duration, Instant};

use super::*;
use crate::app::refresh::RefreshSchedule;

#[test]
fn manual_refresh_notifies_once_after_completion_and_keeps_automatic_refreshes_quiet() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    app.tick(Duration::ZERO, AnimationSettings::default());
    assert_eq!(app.notifications.center().history().len(), 0);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.manual_refresh = Some(receiver);
    app.tick(Duration::ZERO, AnimationSettings::default());
    assert_eq!(app.notifications.center().history().len(), 0);
    sender.send(Ok(())).unwrap();
    app.tick(Duration::ZERO, AnimationSettings::default());
    app.tick(Duration::ZERO, AnimationSettings::default());
    let notifications: Vec<_> = app.notifications.center().history().collect();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].title(), "Refresh complete");
    assert_eq!(notifications[0].kind(), tuicore::NotificationKind::Success);
    assert!(app.manual_refresh.is_none());
}

#[test]
fn manual_refresh_reports_partial_failures_and_worker_shutdown() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    for failure in [Some("Docker unavailable"), None] {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.manual_refresh = Some(receiver);
        if let Some(error) = failure {
            sender.send(Err(error.into())).unwrap();
        } else {
            drop(sender);
        }
        app.tick(Duration::ZERO, AnimationSettings::default());
        let notification = app.notifications.center().history().last().unwrap();
        assert_eq!(notification.kind(), tuicore::NotificationKind::Warning);
        assert_eq!(notification.title(), "Refresh completed with errors");
        assert_eq!(
            notification.body(),
            failure.unwrap_or("Inventory refresh worker stopped")
        );
    }
}

#[test]
fn focused_inventory_refreshes_every_ten_seconds_without_catch_up_bursts() {
    let now = Instant::now();
    let mut schedule = RefreshSchedule::new(now);
    assert!(!schedule.tick(now + Duration::from_secs(9)));
    assert!(schedule.tick(now + Duration::from_secs(10)));
    assert!(!schedule.tick(now + Duration::from_secs(19)));
    assert!(schedule.tick(now + Duration::from_secs(100)));
    assert!(!schedule.tick(now + Duration::from_secs(100)));
}

#[test]
fn unfocused_inventory_refreshes_every_five_minutes_and_focus_refreshes_immediately() {
    let now = Instant::now();
    let mut schedule = RefreshSchedule::new(now);
    assert!(!schedule.event(&TuiEvent::FocusLost, now));
    assert!(!schedule.tick(now + Duration::from_secs(299)));
    assert!(schedule.tick(now + Duration::from_secs(300)));
    assert!(!schedule.tick(now + Duration::from_secs(599)));
    let regained = now + Duration::from_secs(599);
    assert!(schedule.event(&TuiEvent::FocusGained, regained));
    assert!(!schedule.tick(regained + Duration::from_secs(9)));
    assert!(schedule.tick(regained + Duration::from_secs(10)));
}

#[test]
fn changing_widget_focus_does_not_change_terminal_idle_schedule() {
    tuicore::init();
    let mut app = root(AppService::for_tests());
    let mut events = EventCtx::new(AnimationSettings::default());
    app.event(&TuiEvent::FocusLost, &mut events);
    app.handle_message(Msg::OpenSettings, &mut events);
    assert!(
        !app.refresh_schedule
            .tick(Instant::now() + Duration::from_secs(10))
    );
    app.event(&TuiEvent::FocusGained, &mut events);
    assert!(
        app.refresh_schedule
            .tick(Instant::now() + Duration::from_secs(10))
    );
}
