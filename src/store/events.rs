use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(crate) const MAX_EVENT_BYTES: usize = 65_536;
pub(crate) const MAX_BATCH: usize = 100;
pub(crate) const FEED_LIMIT: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Event {
    pub schema_version: u32,
    pub event_id: String,
    pub stream: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub occurred_at: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub people: Vec<Person>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default)]
    pub context: Option<Value>,
    #[serde(flatten)]
    pub payload: Payload,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "profile", content = "data", rename_all = "snake_case")]
pub(crate) enum Payload {
    Message(Message),
    Ticket(Ticket),
    SystemEvent(SystemEvent),
    Generic(Value),
}

impl Payload {
    pub(crate) fn profile(&self) -> &'static str {
        match self {
            Self::Message(_) => "message",
            Self::Ticket(_) => "ticket",
            Self::SystemEvent(_) => "system_event",
            Self::Generic(_) => "generic",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Message {
    pub author: String,
    pub channel: String,
    pub text: String,
    #[serde(default)]
    pub thread: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ticket {
    pub key: String,
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub assignee: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SystemEvent {
    pub resource: String,
    pub signal: String,
    pub severity: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity_color: Option<SeverityColor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SeverityColor {
    Plain,
    Info,
    Warning,
    Error,
    Success,
}

impl SystemEvent {
    pub(crate) fn severity_color(&self) -> SeverityColor {
        if let Some(color) = self.severity_color {
            return color;
        }
        match self.severity.trim().to_ascii_lowercase().as_str() {
            "info" | "information" | "informational" | "notice" | "debug" | "trace" => {
                SeverityColor::Info
            }
            "warn" | "warning" | "caution" => SeverityColor::Warning,
            "err" | "error" | "critical" | "crit" | "fatal" | "severe" | "emergency" | "emerg"
            | "alert" => SeverityColor::Error,
            "success" | "successful" | "ok" | "okay" | "done" | "complete" | "completed"
            | "passed" | "resolved" | "healthy" => SeverityColor::Success,
            _ => SeverityColor::Plain,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Person {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Attachment {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub media_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Relation {
    pub kind: String,
    pub subject: String,
    #[serde(default)]
    pub url: Option<String>,
}

impl Event {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.schema_version != 1 {
            return Err(Error::Invalid("schema_version must be 1".into()));
        }
        for (name, value, maximum) in [
            ("event_id", self.event_id.as_str(), 200),
            ("stream", self.stream.as_str(), 80),
            ("type", self.event_type.as_str(), 200),
            ("summary", self.summary.as_str(), 1000),
        ] {
            validate_text(name, value, maximum)?;
        }
        if let Some(timestamp) = &self.occurred_at {
            chrono::DateTime::parse_from_rfc3339(timestamp)
                .map_err(|_| Error::Invalid("occurred_at must be RFC 3339".into()))?;
        }
        match &self.payload {
            Payload::Message(message) => {
                validate_text("author", &message.author, 200)?;
                validate_text("channel", &message.channel, 200)?;
                if message.text.trim().is_empty() {
                    return Err(Error::Invalid("message text must not be empty".into()));
                }
            }
            Payload::Ticket(ticket) => {
                validate_text("key", &ticket.key, 200)?;
                validate_text("title", &ticket.title, 1000)?;
                validate_text("status", &ticket.status, 200)?;
            }
            Payload::SystemEvent(signal) => {
                validate_text("resource", &signal.resource, 200)?;
                validate_text("signal", &signal.signal, 200)?;
                validate_text("severity", &signal.severity, 80)?;
                if let Some(environment) = &signal.environment
                    && (environment.len() > 200 || environment.chars().any(char::is_control))
                {
                    return Err(Error::Invalid(
                        "environment must be at most 200 bytes and contain no control characters"
                            .into(),
                    ));
                }
                if signal.description.trim().is_empty() {
                    return Err(Error::Invalid(
                        "system description must not be empty".into(),
                    ));
                }
            }
            Payload::Generic(data) if !data.is_object() => {
                return Err(Error::Invalid("generic data must be an object".into()));
            }
            Payload::Generic(_) => {}
        }
        if serde_json::to_vec(self)
            .map_err(|error| Error::Invalid(error.to_string()))?
            .len()
            > MAX_EVENT_BYTES
        {
            return Err(Error::Invalid("event exceeds 64 KiB".into()));
        }
        Ok(())
    }
}

fn validate_text(name: &str, value: &str, maximum: usize) -> Result<(), Error> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(Error::Invalid(format!(
            "{name} must be nonempty, at most {maximum} bytes, and contain no control characters"
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcessingStatus {
    Pending,
    Accepted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Attempt {
    pub id: i64,
    pub status: ProcessingStatus,
    pub replay: bool,
    pub created_at: String,
    pub accepted_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub sequence: i64,
    pub provider: String,
    pub received_at: String,
    pub event: Event,
    pub attempts: Vec<Attempt>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Snapshot {
    pub records: Vec<Record>,
    pub total: u64,
    pub provider_totals: std::collections::BTreeMap<String, u64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Batch {
    pub events: Vec<Event>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub event_id: String,
    pub sequence: i64,
    pub duplicate: bool,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Ingestion {
    pub receipts: Vec<Receipt>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub discarded: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationKind {
    Received,
    Replayed,
    Acknowledged,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ProviderNotification {
    pub notification_id: i64,
    pub event_id: String,
    pub sequence: i64,
    pub attempt_id: i64,
    pub kind: NotificationKind,
    pub created_at: String,
}

#[derive(Debug)]
pub(crate) enum Error {
    Unauthorized,
    Invalid(String),
    Conflict(String),
    NotFound,
    Storage(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => formatter.write_str("invalid provider credential"),
            Self::Invalid(message) | Self::Conflict(message) | Self::Storage(message) => {
                formatter.write_str(message)
            }
            Self::NotFound => formatter.write_str("event or notification not found"),
        }
    }
}

impl std::error::Error for Error {}
