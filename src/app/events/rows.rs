use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};

use super::clean;
use crate::store::events::{Payload, ProcessingStatus, Record, SeverityColor};

pub(crate) fn profile_icon(profile: &str) -> &'static str {
    match profile {
        "message" => "",
        "ticket" => "",
        "system_event" => "",
        _ => "",
    }
}

pub(crate) fn row_text(row: &Record) -> Text<'static> {
    let theme = tuicore::theme();
    let normal = Style::default().fg(theme.text_fg());
    let muted = Style::default().fg(theme.muted_fg());
    let handled = row
        .attempts
        .first()
        .is_some_and(|attempt| attempt.status == ProcessingStatus::Handled);
    let glyph_style = if handled {
        Style::default().fg(theme.success_fg())
    } else {
        normal
    };
    let mut header = vec![
        Span::styled(profile_icon(row.event.payload.profile()), glyph_style),
        Span::styled(" ", normal),
        Span::styled(clean(&row.provider), normal.add_modifier(Modifier::BOLD)),
    ];
    let body = match &row.event.payload {
        Payload::Message(message) => {
            field(&mut header, &message.author, normal, muted);
            field(&mut header, &message.channel, normal, muted);
            if optional(message.thread.as_deref()).is_some() {
                field(&mut header, "󱡠", muted, muted);
            }
            &message.text
        }
        Payload::Ticket(ticket) => {
            field(&mut header, &ticket.key, normal, muted);
            field(&mut header, &ticket.status, normal, muted);
            if let Some(assignee) = optional(ticket.assignee.as_deref()) {
                field(&mut header, &assignee, muted, muted);
            }
            &ticket.title
        }
        Payload::SystemEvent(signal) => {
            field(&mut header, &signal.resource, normal, muted);
            if let Some(environment) = optional(signal.environment.as_deref()) {
                field(&mut header, &environment_label(&environment), muted, muted);
            }
            field(&mut header, &signal.signal, normal, muted);
            let severity_color = match signal.severity_color() {
                SeverityColor::Plain => theme.text_fg(),
                SeverityColor::Info => theme.info_fg(),
                SeverityColor::Warning => theme.warning_fg(),
                SeverityColor::Error => theme.error_fg(),
                SeverityColor::Success => theme.success_fg(),
            };
            field(
                &mut header,
                clean(&signal.severity).trim(),
                Style::default().fg(severity_color),
                muted,
            );
            &signal.description
        }
        Payload::Generic(_) => {
            field(&mut header, &row.event.event_type, muted, muted);
            &row.event.summary
        }
    };
    Text::from(vec![
        Line::from(header),
        Line::from(Span::styled(clean(body), normal)),
    ])
}

fn field(header: &mut Vec<Span<'static>>, value: &str, style: Style, separator: Style) {
    header.push(Span::styled(" · ", separator));
    header.push(Span::styled(clean(value), style));
}

fn optional(value: Option<&str>) -> Option<String> {
    value
        .map(clean)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn environment_label(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "prod" | "production" => "production".into(),
        "dev" | "development" => "development".into(),
        "stage" | "staging" => "staging".into(),
        _ => value.into(),
    }
}
