use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportInput {
    pub title: String,
    pub summary: String,
    /// Full Markdown contents, not a workspace file path.
    pub markdown: String,
}

impl ReportInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, text, limit) in [
            ("title", self.title.as_str(), 500),
            ("summary", self.summary.as_str(), 8_192),
            ("markdown", self.markdown.as_str(), 1_048_576),
        ] {
            if text.trim().is_empty() || text.len() > limit || text.contains('\0') {
                return Err(format!(
                    "{name} must contain 1 to {limit} bytes without NUL"
                ));
            }
        }
        if self.title.chars().any(char::is_control) {
            return Err("title must not contain control characters".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CleanupState {
    Pending,
    Purging,
    Purged,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ReportSummary {
    pub acceptance_id: i64,
    pub event_sequence: i64,
    pub rule_name: String,
    pub title: String,
    pub summary: String,
    pub reported_at: String,
    pub cleanup_state: CleanupState,
    pub cleanup_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum ConclusionReceipt {
    Reported(ReportSummary),
    Unreported {
        instance: String,
        cleanup_state: CleanupState,
    },
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub(crate) struct Report {
    #[serde(flatten)]
    pub details: ReportSummary,
    pub markdown: String,
}

pub(crate) fn search_terms(strings: Vec<String>) -> Result<Vec<String>, String> {
    if strings.is_empty() || strings.len() > 20 {
        return Err("search_strings must contain 1 to 20 strings".into());
    }
    strings
        .into_iter()
        .map(|text| {
            let text = text.trim();
            if text.is_empty() || text.len() > 500 || text.contains('\0') {
                return Err("each search string must contain 1 to 500 bytes without NUL".into());
            }
            Ok(text.to_lowercase())
        })
        .collect()
}
