use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) use crate::store::memories::MemoryInput as ReportInput;

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
pub(crate) struct ConclusionReceipt {
    pub conclusion_id: i64,
    #[serde(default)]
    pub memory_id: Option<i64>,
    pub instance: String,
    pub acceptance_id: Option<i64>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub admitted_at: String,
    pub cleanup_state: CleanupState,
    pub cleanup_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct Conclusion {
    #[serde(flatten)]
    pub details: ConclusionReceipt,
    pub report: Option<ReportInput>,
    #[serde(skip)]
    pub workspace_identity: String,
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
