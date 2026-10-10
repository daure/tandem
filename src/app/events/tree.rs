use ratatui::text::Text;

use super::{Record, search_text};
use crate::app::{
    acceptances::{Context, Target},
    rows::Row,
};

#[derive(Clone, PartialEq)]
pub(super) enum Entry {
    Event(Box<Record>),
    Acceptance(Box<Target>),
    Conversation { target: Box<Target>, row: Box<Row> },
}

impl Entry {
    pub(super) fn id(&self) -> String {
        match self {
            Self::Event(row) => format!("event:{}", row.sequence),
            Self::Acceptance(target) => format!("acceptance:{}", target.acceptance.id),
            Self::Conversation { row, .. } => row.id.clone(),
        }
    }
    pub(super) fn parent(&self) -> Option<String> {
        match self {
            Self::Event(_) => None,
            Self::Acceptance(target) => Some(format!("event:{}", target.acceptance.event_sequence)),
            Self::Conversation { row, .. } => row.parent.clone(),
        }
    }
    pub(super) fn target(&self) -> Option<Target> {
        match self {
            Self::Event(_) => None,
            Self::Acceptance(target) => Some(*target.clone()),
            Self::Conversation { target, row } => {
                let mut target = *target.clone();
                target.selected = Some(*row.clone());
                Some(target)
            }
        }
    }
    pub(super) fn search(&self) -> String {
        match self {
            Self::Event(row) => search_text(row),
            Self::Acceptance(target) => target.search(),
            Self::Conversation { target, row } => {
                format!("{} {}", target.search(), row.search_text())
            }
        }
    }
    pub(super) fn height(&self) -> u16 {
        match self {
            Self::Event(_) => 2,
            Self::Acceptance(target) => target.height(),
            Self::Conversation { row, .. } => row.height(),
        }
    }
    pub(super) fn needs_spinner(&self) -> bool {
        let row = match self {
            Self::Event(_) => None,
            Self::Acceptance(target) => target.instance.as_ref(),
            Self::Conversation { row, .. } => Some(row.as_ref()),
        };
        row.is_some_and(|row| {
            row.loading
                || row.secondary_loading
                || row.metrics.memory_waiting
                || row.metrics.cpu_waiting
        })
    }
    pub(super) fn text(&self, spinner: &str, width: Option<u16>) -> Text<'static> {
        match self {
            Self::Event(row) => super::row_text(row, width),
            Self::Acceptance(target) => target.text(spinner, width),
            Self::Conversation { row, .. } => row.text(spinner, width),
        }
    }
    pub(super) fn memory(&self, spinner: &str) -> ratatui::text::Line<'static> {
        match self {
            Self::Conversation { row, .. } => row.memory_text_with_spinner(spinner),
            _ => ratatui::text::Line::default(),
        }
    }
    pub(super) fn cpu(&self, spinner: &str) -> ratatui::text::Line<'static> {
        match self {
            Self::Conversation { row, .. } => row.cpu_text_with_spinner(spinner),
            _ => ratatui::text::Line::default(),
        }
    }
}

pub(super) fn project(
    records: &[Record],
    rules: &crate::store::rules::Snapshot,
    context: &Context,
    show_saved: bool,
) -> Vec<Entry> {
    let mut rows = Vec::new();
    for record in records {
        rows.push(Entry::Event(Box::new(record.clone())));
        for acceptance in &record.acceptances {
            let acceptance = rules
                .acceptances
                .iter()
                .find(|row| row.id == acceptance.id)
                .unwrap_or(acceptance);
            let target = Target::new(
                acceptance,
                rules.workspaces.get(&acceptance.id),
                context,
                show_saved,
            )
            .with_report(rules.reports.get(&acceptance.id));
            rows.push(Entry::Acceptance(Box::new(target.clone())));
            for row in &target.conversations {
                rows.push(Entry::Conversation {
                    target: Box::new(target.clone()),
                    row: Box::new(row.clone()),
                });
            }
        }
    }
    rows
}
