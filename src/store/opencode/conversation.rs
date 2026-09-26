use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Message {
    pub info: Info,
    pub parts: Vec<Part>,
}

#[derive(Deserialize)]
pub(crate) struct Info {
    pub id: String,
    pub role: String,
    pub agent: Option<String>,
    pub time: MessageTime,
}

#[derive(Deserialize)]
pub(crate) struct MessageTime {
    pub created: u64,
}

#[derive(Deserialize)]
pub(crate) struct Part {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    pub filename: Option<String>,
    pub tool: Option<String>,
    pub state: Option<ToolState>,
}

#[derive(Deserialize)]
pub(crate) struct ToolState {
    pub status: String,
}

pub(crate) fn transcript(mut messages: Vec<Message>) -> String {
    messages.sort_by(|a, b| {
        b.info
            .time
            .created
            .cmp(&a.info.time.created)
            .then_with(|| b.info.id.cmp(&a.info.id))
    });
    let mut sections = Vec::new();
    for message in messages {
        if !matches!(message.info.role.as_str(), "user" | "assistant") {
            continue;
        }
        let parts: Vec<_> = message
            .parts
            .into_iter()
            .filter_map(|part| match part.kind.as_str() {
                "text" if !part.text.trim().is_empty() => Some(safe_text(&part.text)),
                "reasoning" if !part.text.trim().is_empty() => {
                    Some(format!("### Reasoning\n\n{}", safe_text(&part.text)))
                }
                "file" => Some(format!(
                    "Attachment: {}",
                    safe_text(part.filename.as_deref().unwrap_or("file"))
                )),
                "tool" => Some(format!(
                    "Tool: {} ({})",
                    safe_text(part.tool.as_deref().unwrap_or("tool")),
                    safe_text(part.state.as_ref().map_or("unknown", |state| &state.status))
                )),
                _ => None,
            })
            .collect();
        if parts.is_empty() {
            continue;
        }
        let role = if message.info.role == "user" {
            "You".to_owned()
        } else {
            message.info.agent.map_or_else(
                || "Agent".into(),
                |agent| format!("Agent · {}", safe_text(&agent)),
            )
        };
        let timestamp = i64::try_from(message.info.time.created)
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_default();
        sections.push(format!("## {role} · {timestamp}\n\n{}", parts.join("\n\n")));
    }
    if sections.is_empty() {
        return "No messages yet.".into();
    }
    format!(
        "# Conversation\n\nNewest messages first · reopen to refresh\n\n{}",
        sections.join("\n\n---\n\n")
    )
}

pub(crate) fn latest_question(messages: Vec<Message>) -> Option<String> {
    let message = messages
        .into_iter()
        .filter(|message| message.info.role == "user")
        .max_by(|a, b| {
            a.info
                .time
                .created
                .cmp(&b.info.time.created)
                .then_with(|| a.info.id.cmp(&b.info.id))
        })?;
    let question: String = message
        .parts
        .into_iter()
        .filter(|part| part.kind == "text")
        .flat_map(|part| {
            safe_text(&part.text)
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(4096)
        .collect();
    (!question.is_empty()).then_some(question)
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

#[cfg(test)]
#[path = "../tests/opencode_conversation.rs"]
mod tests;
