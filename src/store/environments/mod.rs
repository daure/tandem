use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "Tandem template manifest",
    description = "Configuration for tandem.json. Tandem also validates repository path overlap, source safety and route/one-shot conflicts; Compose service references are checked at instance startup."
)]
pub(crate) struct Manifest {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    /// Public service names mapped to gateway routes and content readiness assertions.
    pub routes: BTreeMap<String, Route>,
    #[serde(default)]
    /// App setup service names; each must match a Compose service and cannot also be a route.
    pub one_shots: Vec<String>,
    #[serde(default)]
    /// Repositories provisioned by Tandem before Compose; target paths must not overlap.
    pub repositories: Vec<Repository>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Repository {
    /// Absolute local path, HTTPS URL without credentials, or ssh:// URL; credentials belong on the host.
    pub source: String,
    /// Workspace-relative path of non-hidden directories. Existing checkouts retain their branch and edits.
    #[schemars(
        length(min = 1, max = 240),
        regex(pattern = r"^[a-zA-Z0-9_-][a-zA-Z0-9_.-]*(/[a-zA-Z0-9_-][a-zA-Z0-9_.-]*)*$")
    )]
    pub target: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Route {
    #[schemars(range(min = 1, max = 65535))]
    pub port: u16,
    #[serde(default = "default_strip")]
    pub strip_prefix: bool,
    /// Relative to the service URL; an empty string probes the route root.
    pub readiness_path: String,
    /// Non-whitespace content required in a successful readiness response; at most 4096 UTF-8 bytes.
    #[schemars(length(min = 1, max = 4096))]
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
    pub manifest_source: Option<String>,
    pub manifest: Manifest,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct InstanceService {
    pub name: String,
    pub container_id: String,
    pub status: String,
    pub one_shot: bool,
    pub image: Option<String>,
    pub health: Option<String>,
    pub restart_policy: Option<String>,
    pub restart_count: u64,
    pub created_at: Option<String>,
    pub started_at: Option<String>,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub usage: Option<ResourceUsage>,
    pub memory_limit_bytes: Option<u64>,
    pub volumes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct ResourceUsage {
    pub cpu_basis_points: Option<u64>,
    pub memory_bytes: u64,
    pub sampled_at_unix_seconds: u64,
}

impl ResourceUsage {
    pub fn total<'a>(services: impl Iterator<Item = &'a InstanceService>) -> Option<Self> {
        let mut total: Option<Self> = None;
        for service in services.filter(|service| service.consumes_resources()) {
            let usage = service.usage?;
            total = Some(match total {
                None => usage,
                Some(previous) => Self {
                    cpu_basis_points: previous
                        .cpu_basis_points
                        .zip(usage.cpu_basis_points)
                        .and_then(|(previous, current)| previous.checked_add(current)),
                    memory_bytes: previous.memory_bytes.checked_add(usage.memory_bytes)?,
                    sampled_at_unix_seconds: previous
                        .sampled_at_unix_seconds
                        .min(usage.sampled_at_unix_seconds),
                },
            });
        }
        total
    }
}

impl InstanceService {
    pub fn total_memory_limit<'a>(services: impl Iterator<Item = &'a Self>) -> Option<u64> {
        let mut total = 0_u64;
        for service in services.filter(|service| service.consumes_resources()) {
            total = total.checked_add(service.memory_limit_bytes.filter(|limit| *limit > 0)?)?;
        }
        (total > 0).then_some(total)
    }

    pub fn consumes_resources(&self) -> bool {
        matches!(
            self.status.as_str(),
            "up" | "healthy" | "unhealthy" | "boot" | "paused" | "removing"
        )
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct Instance {
    pub name: String,
    pub template: String,
    pub template_directory: String,
    pub workspace: String,
    pub project: String,
    pub pending: bool,
    pub services: Vec<InstanceService>,
}

impl Instance {
    pub fn startup_error(&self) -> Option<String> {
        self.services
            .iter()
            .find(|service| service.one_shot && service.status.starts_with("down"))
            .map(|service| {
                format!(
                    "{}: {}; inspect its container logs",
                    service.name, service.status
                )
            })
    }
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct EnvironmentSnapshot {
    pub templates: Vec<Template>,
    pub instances: Vec<Instance>,
    pub startup: BTreeMap<String, StartupTiming>,
    pub startup_averages_milliseconds: BTreeMap<String, u64>,
    pub templates_root: String,
    pub home_directory: Option<String>,
    pub gateway_origin: String,
    pub error: Option<String>,
    pub loading: bool,
    pub resource_error: Option<String>,
    pub resource_sample_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema, PartialEq, Eq)]
pub(crate) struct StartupTiming {
    pub elapsed_milliseconds: u64,
    pub estimate_milliseconds: Option<u64>,
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
    pub service: Option<String>,
    pub state: OperationState,
    pub progress: Vec<String>,
    pub elapsed_seconds: u64,
    pub elapsed_milliseconds: u64,
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
    pub manifest_schema: serde_json::Value,
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

pub(crate) fn validate_instance_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 40
        || !name.as_bytes()[0].is_ascii_alphabetic() && !name.as_bytes()[0].is_ascii_digit()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte.is_ascii_digit() || byte == b'-')
        || name.eq_ignore_ascii_case("gateway")
    {
        return Err("name must be 1–40 letters, digits or hyphens, start with a letter/digit, and not be 'gateway'".into());
    }
    Ok(())
}
