use ratatui::{
    style::Style,
    text::{Line, Span, Text},
};

use crate::store::providers::{Action, Provider, Snapshot, Status, Stream};

#[derive(Clone)]
pub(super) struct Row {
    pub id: String,
    pub parent: Option<String>,
    pub provider: Provider,
    pub stream: Option<Stream>,
}

pub(super) fn from_snapshot(snapshot: &Snapshot) -> Vec<Row> {
    let mut rows = Vec::new();
    for provider in &snapshot.providers {
        rows.push(Row {
            id: provider.name.clone(),
            parent: None,
            provider: provider.clone(),
            stream: None,
        });
        for stream in &provider.streams {
            rows.push(Row {
                id: format!("{}\u{1f}{}", provider.name, stream.name),
                parent: Some(provider.name.clone()),
                provider: provider.clone(),
                stream: Some(stream.clone()),
            });
        }
    }
    rows
}

pub(super) fn search_text(row: &Row) -> String {
    format!(
        "{} {} {}",
        super::provider_label(&row.provider),
        row.provider
            .manifest
            .as_ref()
            .map(|manifest| manifest.description.as_str())
            .unwrap_or(""),
        row.stream
            .as_ref()
            .map(|stream| stream.name.as_str())
            .unwrap_or("")
    )
}

pub(super) fn stream_text(stream: &Stream) -> Text<'static> {
    let theme = tuicore::theme();
    let (status, color) = match (stream.operation, stream.status) {
        (Some(Action::Start), _) => ("Starting...", theme.info_fg()),
        (Some(Action::Stop), _) => ("Stopping...", theme.info_fg()),
        (_, Status::Starting) => ("Starting...", theme.info_fg()),
        (_, Status::Running) => ("Collecting", theme.success_fg()),
        (_, Status::Stopped) => ("Stopped", theme.muted_fg()),
        (_, Status::NotStarted) => ("Not started", theme.muted_fg()),
        (_, Status::Paused) => ("Provider paused", theme.warning_fg()),
        (_, Status::Unknown) if !stream.controllable => {
            ("Observed · provider controls only", theme.muted_fg())
        }
        _ => ("Unverified", theme.warning_fg()),
    };
    Text::from(Line::from(vec![
        Span::styled(
            crate::app::events::profile_icon(stream.profile.as_deref().unwrap_or("generic")),
            Style::default().fg(color),
        ),
        Span::raw(format!(
            " {} ·  {}/{} · ",
            super::clean(&stream.name),
            stream.handovers,
            stream.total
        )),
        Span::styled(status, Style::default().fg(color)),
    ]))
}
