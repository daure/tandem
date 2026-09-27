use std::{collections::HashMap, time::Duration};

use ratatui::style::{Color, Style};
use tuicore::{AnimationSettings, Easing, TickResult, lerp_color};

use crate::{
    app::{opencode::Target, rows::Row},
    store::opencode::Activity,
};

const PULSE_DURATION: Duration = Duration::from_millis(300);
const PULSE_COUNT: u32 = 2;
const PEAK_TINT: f64 = 0.20;

#[derive(Default)]
pub(super) struct Pulses {
    elapsed: HashMap<String, Duration>,
}

fn session(row: &Row) -> Option<(&str, Activity)> {
    let Target::Session { id, .. } = row.opencode.as_ref()? else {
        return None;
    };
    Some((id, row.opencode_activity?))
}

impl Pulses {
    pub(super) fn observe(&mut self, previous: &[Row], current: &[Row], reset: bool) {
        if reset {
            self.elapsed.clear();
            return;
        }
        let previous: HashMap<_, _> = previous.iter().filter_map(session).collect();
        let current: HashMap<_, _> = current.iter().filter_map(session).collect();
        self.elapsed
            .retain(|id, _| current.get(id.as_str()) == Some(&Activity::Idle));
        for (id, activity) in current {
            if activity == Activity::Idle && previous.get(id) == Some(&Activity::Busy) {
                self.elapsed.insert(id.to_owned(), Duration::ZERO);
            }
        }
    }

    pub(super) fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        if self.elapsed.is_empty() {
            return TickResult::IDLE;
        }
        if !settings.enabled {
            self.elapsed.clear();
            return TickResult::CHANGED;
        }
        self.elapsed.retain(|_, elapsed| {
            *elapsed = elapsed.saturating_add(dt.min(settings.max_dt));
            *elapsed < PULSE_DURATION * PULSE_COUNT
        });
        TickResult {
            active: !self.elapsed.is_empty(),
            ..TickResult::CHANGED
        }
    }

    pub(super) fn style(&self, row: &Row, base: Style) -> Style {
        let Some((id, Activity::Idle)) = session(row) else {
            return base;
        };
        let Some(elapsed) = self.elapsed.get(id) else {
            return base;
        };
        let phase = (elapsed.as_secs_f64() / PULSE_DURATION.as_secs_f64()).fract();
        let strength = Easing::EaseInOut.apply(1.0 - (2.0 * phase - 1.0).abs()) * PEAK_TINT;
        if strength == 0.0 {
            return base;
        }
        let theme = tuicore::theme();
        let background = base.bg.unwrap_or_else(|| theme.background_bg());
        // A terminal-default background has no RGB value to interpolate.
        let background = match background {
            Color::Rgb(..) => background,
            _ => theme.dialog_bg(),
        };
        base.bg(lerp_color(background, theme.success_fg(), strength))
    }
}
