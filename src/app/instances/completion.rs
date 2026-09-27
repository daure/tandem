use std::{collections::HashMap, time::Duration};

use ratatui::style::{Color, Style};
use ratatui::text::Span;
use tuicore::{
    AnimationSettings, Easing, EventCtx, EventOutcome, KeySpec, TickResult, TuiEvent, lerp_color,
};

use crate::{
    app::{Msg, opencode::Target, rows::Row},
    store::opencode::Activity,
};

const PULSE_DURATION: Duration = Duration::from_millis(300);
const PULSE_COUNT: u32 = 2;
const FINAL_RISE_DURATION: Duration = Duration::from_millis(150);
const PEAK_TINT: f64 = 0.20;

pub(super) struct Markers {
    elapsed: HashMap<String, Duration>,
    fade_duration: Duration,
}

fn session(row: &Row) -> Option<(&str, Activity)> {
    let Target::Session { id, .. } = row.opencode.as_ref()? else {
        return None;
    };
    Some((id, row.opencode_activity?))
}

impl Markers {
    pub(super) fn new(fade_duration: Duration) -> Self {
        Self {
            elapsed: HashMap::new(),
            fade_duration,
        }
    }

    pub(super) fn observe(&mut self, previous: &[Row], current: &[Row], reset: bool) {
        if reset {
            self.elapsed.clear();
            return;
        }
        let previous: HashMap<_, _> = previous.iter().filter_map(session).collect();
        let current: HashMap<_, _> = current.iter().filter_map(session).collect();
        self.elapsed.retain(|id, _| {
            current
                .get(id.as_str())
                .is_some_and(|activity| activity.completed())
        });
        for (id, activity) in current {
            if activity.completed() && previous.get(id) == Some(&Activity::Busy) {
                self.elapsed.insert(id.to_owned(), Duration::ZERO);
            }
        }
    }

    pub(super) fn tick(
        &mut self,
        dt: Duration,
        settings: AnimationSettings,
        fade_duration: Duration,
    ) -> TickResult {
        self.fade_duration = fade_duration;
        if self.elapsed.is_empty() {
            return TickResult::IDLE;
        }
        if !settings.enabled {
            self.elapsed.clear();
            return TickResult::CHANGED;
        }
        self.elapsed.retain(|_, elapsed| {
            *elapsed = elapsed.saturating_add(dt);
            *elapsed < PULSE_DURATION * PULSE_COUNT + FINAL_RISE_DURATION + self.fade_duration
        });
        TickResult {
            active: !self.elapsed.is_empty(),
            ..TickResult::CHANGED
        }
    }

    fn next(&self, rows: &[Row], highlighted: Option<&str>, backwards: bool) -> Option<String> {
        if self.elapsed.is_empty() {
            return None;
        }
        let mut children: HashMap<Option<&str>, Vec<&Row>> = HashMap::new();
        for row in rows {
            children.entry(row.parent.as_deref()).or_default().push(row);
        }
        let mut pending = children.remove(&None).unwrap_or_default();
        pending.reverse();
        let mut ordered = Vec::with_capacity(rows.len());
        while let Some(row) = pending.pop() {
            ordered.push(row);
            if let Some(children) = children.remove(&Some(row.id.as_str())) {
                pending.extend(children.into_iter().rev());
            }
        }
        if backwards {
            ordered.reverse();
        }
        let start = ordered
            .iter()
            .position(|row| Some(row.id.as_str()) == highlighted)
            .map_or(0, |index| index + 1);
        ordered.into_iter().skip(start).find_map(|row| {
            let (id, activity) = session(row)?;
            (activity.completed() && self.elapsed.contains_key(id)).then(|| row.id.clone())
        })
    }

    pub(super) fn marker(&self, row: &Row, base: Style) -> Option<Span<'static>> {
        let Some((id, Activity::Idle | Activity::AwaitingAnswer)) = session(row) else {
            return None;
        };
        let elapsed = self.elapsed.get(id)?;
        let strength = if *elapsed < PULSE_DURATION * PULSE_COUNT {
            pulse_strength(*elapsed)
        } else {
            let final_elapsed = *elapsed - PULSE_DURATION * PULSE_COUNT;
            if final_elapsed < FINAL_RISE_DURATION {
                Easing::EaseInOut
                    .apply(final_elapsed.as_secs_f64() / FINAL_RISE_DURATION.as_secs_f64())
            } else {
                1.0 - (final_elapsed - FINAL_RISE_DURATION).as_secs_f64()
                    / self.fade_duration.as_secs_f64()
            }
        };
        let theme = tuicore::theme();
        Some(Span::styled(
            "┃",
            Style::default().fg(lerp_color(background(base), theme.success_fg(), strength)),
        ))
    }

    pub(super) fn style(&self, row: &Row, base: Style) -> Style {
        let Some((id, Activity::Idle | Activity::AwaitingAnswer)) = session(row) else {
            return base;
        };
        let Some(elapsed) = self.elapsed.get(id) else {
            return base;
        };
        if *elapsed >= PULSE_DURATION * PULSE_COUNT {
            return base;
        }
        let strength = pulse_strength(*elapsed) * PEAK_TINT;
        if strength == 0.0 {
            return base;
        }
        base.bg(lerp_color(
            background(base),
            tuicore::theme().success_fg(),
            strength,
        ))
    }
}

impl super::Instances {
    pub(super) fn navigate_completion(
        &mut self,
        event: &TuiEvent,
        ctx: &mut EventCtx<Msg>,
    ) -> Option<EventOutcome> {
        if self.tree.is_searching() {
            return None;
        }
        let TuiEvent::Key(key) = event else {
            return None;
        };
        let backwards = if KeySpec::shifted('j').matches(*key) {
            false
        } else if KeySpec::shifted('k').matches(*key) {
            true
        } else {
            return None;
        };
        let target = self.completion.borrow().next(
            self.tree.rows(),
            self.tree.highlighted_id().as_deref(),
            backwards,
        );
        if let Some(id) = target {
            self.tree.clear_search();
            let mut current = id.clone();
            while let Some(parent) = self
                .tree
                .rows()
                .iter()
                .find(|row| row.id == current)
                .and_then(|row| row.parent.clone())
            {
                self.tree.expand(&parent);
                current = parent;
            }
            self.tree.highlight_id(&id);
            self.tree.reveal_highlighted();
            self.after_event();
            ctx.request_layout();
            ctx.request_redraw();
        }
        ctx.stop_propagation();
        Some(EventOutcome::Handled)
    }
}

fn pulse_strength(elapsed: Duration) -> f64 {
    let phase =
        (elapsed.as_nanos() % PULSE_DURATION.as_nanos()) as f64 / PULSE_DURATION.as_nanos() as f64;
    Easing::EaseInOut.apply(1.0 - (2.0 * phase - 1.0).abs())
}

fn background(base: Style) -> Color {
    let theme = tuicore::theme();
    let background = base.bg.unwrap_or_else(|| theme.background_bg());
    // A terminal-default background has no RGB value to interpolate.
    match background {
        Color::Rgb(..) => background,
        _ => theme.dialog_bg(),
    }
}
