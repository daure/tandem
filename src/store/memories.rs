use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Link {
    pub label: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryInput {
    pub title: String,
    pub summary: String,
    /// Full text, not a file path. Supply Markdown, nonempty fields, or both.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub markdown: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}

impl MemoryInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        text("title", &self.title, 500)?;
        text("summary", &self.summary, 8_192)?;
        if self.title.chars().any(char::is_control) {
            return Err("title must not contain control characters".into());
        }
        if self.markdown.trim().is_empty() && self.fields.is_empty() {
            return Err("supply nonempty markdown, fields, or both".into());
        }
        if self.labels.len() > 32 || self.links.len() > 32 {
            return Err("memories allow at most 32 labels and 32 links".into());
        }
        for label in &self.labels {
            text("label", label, 128)?;
            if label.chars().any(char::is_control) {
                return Err("labels must not contain control characters".into());
            }
        }
        for link in &self.links {
            text("link label", &link.label, 200)?;
            canonical_url(&link.url)?;
        }
        let payload = serde_json::to_string(self).map_err(|error| error.to_string())?;
        if payload.len() > 1_048_576 {
            return Err("memory content must fit within 1 MiB of JSON".into());
        }
        let mut nodes = 0;
        validate_json(
            &serde_json::to_value(self).map_err(|error| error.to_string())?,
            0,
            &mut nodes,
        )
    }

    pub(crate) fn normalized(mut self) -> Result<Self, String> {
        self.validate()?;
        self.labels = self
            .labels
            .into_iter()
            .map(|label| label.trim().to_lowercase())
            .collect();
        self.labels.sort();
        self.labels.dedup();
        for link in &mut self.links {
            link.url = canonical_url(&link.url)?;
        }
        for value in self.fields.values_mut() {
            value.sort_all_objects();
        }
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn rendered_markdown(&self) -> String {
        let mut markdown = self.markdown.clone();
        if !self.fields.is_empty() {
            markdown.push_str("\n\n## Structured evidence\n\n```json\n");
            markdown.push_str(
                &serde_json::to_string_pretty(&self.fields).expect("JSON fields serialize"),
            );
            markdown.push_str("\n```\n");
        }
        if !self.labels.is_empty() {
            markdown.push_str(&format!("\n\nLabels: {}\n", self.labels.join(", ")));
        }
        for link in &self.links {
            markdown.push_str(&format!("\n{}: {}\n", link.label, link.url));
        }
        markdown
    }
}

fn text(name: &str, value: &str, limit: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > limit || value.contains('\0') {
        return Err(format!(
            "{name} must contain 1 to {limit} bytes without NUL"
        ));
    }
    Ok(())
}

fn validate_json(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
    *nodes += 1;
    if depth > 16 || *nodes > 4_096 {
        return Err("memory JSON exceeds 16 levels or 4096 values".into());
    }
    match value {
        Value::String(value) if value.contains('\0') => {
            return Err("memory strings must not contain NUL".into());
        }
        Value::Array(values) => {
            for value in values {
                validate_json(value, depth + 1, nodes)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                text("field name", key, 500)?;
                validate_json(value, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn canonical_url(value: &str) -> Result<String, String> {
    text("link URL", value, 2_048)?;
    let url =
        reqwest::Url::parse(value).map_err(|_| "link URL must be absolute HTTP(S)".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("link URL must be absolute HTTP(S) without credentials".into());
    }
    Ok(url.to_string())
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct MemorySummary {
    pub memory_id: i64,
    pub instance: String,
    /// Opaque workspace incarnation; empty for legacy acceptance-only reports.
    pub workspace_identity: String,
    pub acceptance_id: Option<i64>,
    pub conclusion_id: Option<i64>,
    pub created_at: String,
    pub title: String,
    pub summary: String,
    pub labels: Vec<String>,
    pub links: Vec<Link>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct Memory {
    #[serde(flatten)]
    pub details: MemorySummary,
    pub markdown: String,
    pub fields: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct MemoryMatch {
    #[serde(flatten)]
    pub details: MemorySummary,
    pub snippet: String,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemorySearch {
    /// Plain words, all required. English stemming; prefixes apply to normalized tokens. No query operators.
    pub query: Option<String>,
    /// All labels required; case-insensitive exact matches after trimming.
    #[serde(default)]
    pub labels: Vec<String>,
    /// All canonical HTTP(S) URLs required; never fetched.
    #[serde(default)]
    pub links: Vec<String>,
    pub instance: Option<String>,
    /// Inclusive RFC3339 timestamp.
    pub after: Option<String>,
    /// Inclusive RFC3339 timestamp.
    pub before: Option<String>,
    /// All named top-level fields must equal these JSON values, including type.
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    /// 1–100 results; defaults to 20.
    pub limit: Option<u32>,
    /// Skip this many ranked results; defaults to zero.
    pub offset: Option<u32>,
}

impl MemorySearch {
    pub(crate) fn validate(&self) -> Result<Option<String>, String> {
        if !(1..=100).contains(&self.limit.unwrap_or(20)) || self.offset.unwrap_or(0) > 10_000 {
            return Err("limit must be 1–100; offset must be at most 10000".into());
        }
        if self.labels.len() > 32 || self.links.len() > 32 || self.fields.len() > 32 {
            return Err("search allows at most 32 filters of each kind".into());
        }
        for label in &self.labels {
            text("label", label, 128)?;
        }
        for link in &self.links {
            canonical_url(link)?;
        }
        if let Some(instance) = &self.instance {
            crate::store::environments::validate_instance_name(instance)?;
        }
        let after = self.after.as_deref().map(parse_time).transpose()?;
        let before = self.before.as_deref().map(parse_time).transpose()?;
        if after
            .zip(before)
            .is_some_and(|(after, before)| after > before)
        {
            return Err("after must not be later than before".into());
        }
        let filters = serde_json::to_value(&self.fields).map_err(|error| error.to_string())?;
        validate_json(&filters, 0, &mut 0)?;
        if filters.to_string().len() > 16_384 {
            return Err("field filters must fit within 16 KiB".into());
        }
        self.query
            .as_ref()
            .map(|query| {
                text("query", query, 2_000)?;
                let words: Vec<_> = query
                    .split(|character: char| !character.is_alphanumeric())
                    .filter(|word| !word.is_empty())
                    .collect();
                if words.is_empty() || words.len() > 32 {
                    return Err("query must contain 1–32 words".into());
                }
                Ok(words
                    .into_iter()
                    .map(|word| format!("\"{word}\"*"))
                    .collect::<Vec<_>>()
                    .join(" AND "))
            })
            .transpose()
    }
}

pub(crate) fn parse_time(value: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| time.timestamp_millis())
        .map_err(|_| "date filters must be RFC3339 timestamps".into())
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryLookup {
    pub memory_id: i64,
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct MemoryMatches {
    pub memories: Vec<MemoryMatch>,
    pub has_more: bool,
}
