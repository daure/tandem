use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Sessions {
    #[default]
    None,
    Current,
    All,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloseInput {
    #[serde(default)]
    pub sessions: Sessions,
    #[serde(default)]
    pub zellij_tab: bool,
}

impl CloseInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.sessions == Sessions::None && !self.zellij_tab {
            return Err("close requires sessions=current/all or zellij_tab=true".into());
        }
        Ok(())
    }
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct CloseReceipt {
    pub instance: String,
    pub sessions: Sessions,
    pub zellij_tab: bool,
    pub state: &'static str,
}
