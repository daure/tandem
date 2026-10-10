use serde::{Deserialize, Serialize};

use super::Attempt;
use crate::store::{
    environments::OperationState,
    rules::{Acceptance, Rule},
};

pub(crate) const ATTEMPT_LIMIT: usize = 50;
pub(crate) const LOG_LINE_LIMIT: usize = 100;
pub(crate) const LOG_BYTE_LIMIT: usize = 16_384;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Counts {
    pub errors: u64,
    pub warnings: u64,
}

impl Counts {
    pub(crate) fn has_issues(&self) -> bool {
        self.errors > 0 || self.warnings > 0
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Diagnostics {
    pub sequence: i64,
    pub event_id: String,
    pub provider: String,
    pub summary: String,
    pub counts: Counts,
    pub attempts_total: u64,
    pub attempts_truncated: bool,
    pub attempts: Vec<AttemptDiagnostics>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AttemptDiagnostics {
    pub attempt: Attempt,
    pub rules: Vec<RuleDiagnostics>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvaluationOutcome {
    Pending,
    Matched,
    NoMatch,
    Failed,
    Deferred,
    Throttled,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RuleDiagnostics {
    pub rule: Rule,
    pub outcome: EvaluationOutcome,
    pub error: Option<String>,
    pub warning: Option<String>,
    pub acceptance: Option<Acceptance>,
    pub startup: Option<StartupDiagnostics>,
    pub details_unavailable: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct StartupDiagnostics {
    pub operation_id: String,
    pub state: OperationState,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub progress: Vec<String>,
    pub logs_truncated: bool,
}
