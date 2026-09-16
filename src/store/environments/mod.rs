use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub routes: BTreeMap<String, Route>,
    #[serde(default)]
    pub one_shots: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Route {
    pub port: u16,
    #[serde(default = "default_strip")]
    pub strip_prefix: bool,
    pub readiness_path: String,
    pub readiness_contains: String,
}

fn default_strip() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct Template {
    pub name: String,
    pub directory: String,
    pub compose_file: String,
    pub manifest_file: String,
    pub compose_source: String,
    pub manifest: Manifest,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct InstanceService {
    pub name: String,
    pub container_id: String,
    pub status: String,
    pub one_shot: bool,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct Instance {
    pub name: String,
    pub template: String,
    pub template_directory: String,
    pub workspace: String,
    pub project: String,
    pub services: Vec<InstanceService>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct EnvironmentSnapshot {
    pub templates: Vec<Template>,
    pub instances: Vec<Instance>,
    pub templates_root: String,
    pub gateway_origin: String,
    pub error: Option<String>,
    pub loading: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationState {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct Operation {
    pub id: String,
    pub action: String,
    pub name: String,
    pub template: Option<String>,
    pub state: OperationState,
    pub progress: Vec<String>,
    pub elapsed_seconds: u64,
    pub error: Option<String>,
    pub instance: Option<Instance>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(crate) struct Instructions {
    pub file: String,
    pub markdown: String,
    pub templates_root: String,
    pub workspaces_root: String,
    pub gateway_origin: String,
}

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 40
        || !name.as_bytes()[0].is_ascii_lowercase() && !name.as_bytes()[0].is_ascii_digit()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || name == "gateway"
    {
        return Err("name must be 1–40 lowercase letters, digits or hyphens, start with a letter/digit, and not be 'gateway'".into());
    }
    Ok(())
}
