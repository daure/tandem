use std::{collections::BTreeMap, time::Duration};

use tuicore::{AnimationSettings, RelativeDate, TickResult, TuiNode};

use super::{Msg, Record};

#[derive(Default)]
pub(super) struct RelativeTimes {
    dates: BTreeMap<i64, RelativeDate>,
}

impl RelativeTimes {
    pub(super) fn sync(&mut self, records: &[Record]) {
        self.dates
            .retain(|sequence, _| records.iter().any(|row| row.sequence == *sequence));
        for row in records {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.dates.entry(row.sequence)
                && let Ok(date) = chrono::DateTime::parse_from_rfc3339(&row.received_at)
                && let Ok(target) = time::OffsetDateTime::from_unix_timestamp(date.timestamp())
            {
                entry.insert(RelativeDate::new(target));
            }
        }
    }

    pub(super) fn text(&self, sequence: i64) -> Option<&str> {
        self.dates.get(&sequence).map(RelativeDate::text)
    }

    pub(super) fn tick(&mut self, dt: Duration, settings: AnimationSettings) -> TickResult {
        self.dates
            .values_mut()
            .fold(TickResult::IDLE, |result, date| {
                result.merge(<RelativeDate as TuiNode<Msg>>::tick(date, dt, settings))
            })
    }
}

#[cfg(test)]
#[path = "tests/relative_times.rs"]
mod tests;
