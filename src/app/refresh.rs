use std::time::{Duration, Instant};

use tuicore::TuiEvent;

impl super::App {
    pub(crate) fn notify(&mut self, notification: tuicore::Notification) {
        self.notifications.push(notification);
    }

    pub(super) fn poll_manual_refresh(&mut self) {
        use tokio::sync::oneshot::error::TryRecvError;
        let Some(reply) = &mut self.manual_refresh else {
            return;
        };
        let result = match reply.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Closed) => Err("Inventory refresh worker stopped".into()),
        };
        self.manual_refresh = None;
        self.notify(match result {
            Ok(()) => tuicore::Notification::success(
                "Refresh complete",
                "Templates, instances and settings refreshed.",
            ),
            Err(error) => tuicore::Notification::warning("Refresh completed with errors", error),
        });
    }
}

pub(super) struct RefreshSchedule {
    focused: bool,
    next_refresh: Instant,
}

impl Default for RefreshSchedule {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl RefreshSchedule {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            focused: true,
            next_refresh: now + Duration::from_secs(10),
        }
    }

    pub(super) fn event(&mut self, event: &TuiEvent, now: Instant) -> bool {
        match event {
            TuiEvent::FocusGained => {
                self.focused = true;
                self.next_refresh = now + self.interval();
                true
            }
            TuiEvent::FocusLost => {
                self.focused = false;
                self.next_refresh = now + self.interval();
                false
            }
            _ => false,
        }
    }

    pub(super) fn tick(&mut self, now: Instant) -> bool {
        // Animation tick deltas are capped by tuicore; inventory uses wall-clock deadlines.
        if now < self.next_refresh {
            return false;
        }
        self.next_refresh = now + self.interval();
        true
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(if self.focused { 10 } else { 300 })
    }
}
