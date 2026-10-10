use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::events::Event;

mod instance;
mod prompt;
pub(crate) use instance::{
    InstanceOverrides, default_instance_description, resolve_instance_hooks,
};
pub(crate) mod reports;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Definition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub script: String,
    pub template: String,
    pub model: String,
    /// Optional model-specific thinking variant; omitted leaves OpenCode's default selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub initial_prompt: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "super::environments::Instance::default_start_instance")]
    pub start_instance: bool,
    /// Whether automatic acceptance launches focus their new OpenCode pane.
    #[serde(default = "Definition::default_focus_pane")]
    pub focus_pane: bool,
    #[serde(default)]
    pub throttle_seconds: u32,
    #[serde(default)]
    pub trigger_at_end: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub(crate) struct Rule {
    pub definition: Definition,
    pub revision: i64,
    pub zellij_session: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DispatchStatus {
    Queued,
    Provisioning,
    Launching,
    Launched,
    Failed,
    Uncertain,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Acceptance {
    pub id: i64,
    pub event_sequence: i64,
    pub event_summary: String,
    pub attempt_id: i64,
    pub rule_name: String,
    pub rule_revision: i64,
    pub accepted_at: String,
    pub instance: String,
    pub session_id: Option<String>,
    pub pane: Option<super::opencode::Pane>,
    pub operation_id: Option<String>,
    pub launch_started_at: Option<String>,
    pub status: DispatchStatus,
    pub error: Option<String>,
    pub rule: Rule,
    pub resolved_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_description: Option<String>,
}

impl Acceptance {
    pub(crate) fn instance_description(&self) -> String {
        self.resolved_description
            .clone()
            .unwrap_or_else(|| default_instance_description(&self.rule_name, &self.event_summary))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Evaluation {
    pub attempt_id: i64,
    pub rule_name: String,
    pub rule_revision: i64,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Snapshot {
    pub rules: Vec<Rule>,
    pub acceptances: Vec<Acceptance>,
    pub evaluation_errors: Vec<Evaluation>,
    pub evaluation_warnings: Vec<Evaluation>,
    pub error: Option<String>,
    pub workspaces: std::collections::BTreeMap<i64, AcceptanceWorkspace>,
    pub reports: std::collections::BTreeMap<i64, reports::ReportSummary>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct AcceptanceWorkspace {
    pub directory: String,
    pub sessions: Vec<super::opencode::Session>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub removed_sessions: std::collections::BTreeSet<(String, String)>,
}

impl AcceptanceWorkspace {
    pub(crate) fn edit_sessions(
        &mut self,
        edits: &std::collections::BTreeMap<(String, String), Option<String>>,
    ) {
        if !self
            .sessions
            .iter()
            .any(|session| edits.contains_key(&(session.server.clone(), session.id.clone())))
        {
            return;
        }
        self.sessions.retain_mut(|session| {
            match edits.get(&(session.server.clone(), session.id.clone())) {
                Some(Some(title)) => session.title.clone_from(title),
                Some(None) => return false,
                None => {}
            }
            true
        });
        self.removed_sessions.extend(
            edits
                .iter()
                .filter_map(|(key, title)| title.is_none().then_some(key.clone())),
        );
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Preview {
    pub matched: bool,
    pub resolved_prompt: Option<String>,
    pub template: String,
    pub model: String,
    pub variant: Option<String>,
    pub instance_name: Option<String>,
    pub instance_description: Option<String>,
    pub name_is_advisory: bool,
}

fn engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    engine
        .set_max_operations(50_000)
        .set_max_call_levels(32)
        .set_max_expr_depths(64, 32)
        .set_max_string_size(131_072)
        .set_max_array_size(4096)
        .set_max_map_size(4096)
        .disable_symbol("eval")
        .disable_symbol("import")
        .on_print(|_| {})
        .on_debug(|_, _, _| {});
    engine.register_fn("sample", sample);
    engine
}

fn sample(key: &str, probability: f64) -> Result<bool, Box<rhai::EvalAltResult>> {
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        return Err("sample probability must be between 0.0 and 1.0".into());
    }
    if key.is_empty() || key.len() > 1024 {
        return Err("sample key must contain 1 to 1024 bytes".into());
    }
    // Fixed hashing keeps previews, retries, and process restarts on the same draw.
    let mut hash = key.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
    hash ^= hash >> 31;
    let draw = (hash >> 11) as f64 / (1u64 << 53) as f64;
    Ok(draw < probability)
}

fn compile(engine: &rhai::Engine, script: &str) -> Result<rhai::AST, String> {
    let ast = engine.compile(script).map_err(|error| error.to_string())?;
    if !ast
        .iter_functions()
        .any(|function| function.name == "matches" && function.params.len() == 1)
    {
        return Err("script must define fn matches(event)".into());
    }
    Ok(ast.clone_functions_only())
}

pub(crate) fn validate(definition: &Definition) -> Result<(), String> {
    super::environments::validate_name(&definition.name)?;
    super::environments::validate_name(&definition.template)?;
    if definition.description.len() > 1000 || definition.description.chars().any(char::is_control) {
        return Err("description must be at most 1000 bytes without control characters".into());
    }
    definition.session_launch().validate()?;
    if definition.script.len() > 32_768
        || definition.initial_prompt.trim().is_empty()
        || definition.initial_prompt.contains('\0')
        || definition.initial_prompt.len() > 65_536
    {
        return Err(
            "rule exceeds script (32 KiB) or prompt (64 KiB) limits, or has an empty prompt".into(),
        );
    }
    compile(&engine(), &definition.script)?;
    validate_prompt(&definition.initial_prompt)
}

pub(crate) fn validate_prompt(source: &str) -> Result<(), String> {
    prompt::compile(source).map(|_| ())
}

impl Definition {
    fn default_focus_pane() -> bool {
        true
    }

    pub(crate) fn session_launch(&self) -> super::opencode::Launch {
        super::opencode::Launch {
            model: Some(self.model.clone()),
            variant: self.variant.clone(),
            prompt: None,
        }
    }
}

pub(crate) fn instance_name(rule: &str, sequence: i64, acceptance_id: i64) -> String {
    let mut suffix = format!("-{sequence}-a{acceptance_id}");
    if suffix.len() >= 40 {
        suffix = format!("-a{acceptance_id}");
    }
    let prefix = &rule[..rule.len().min(40 - suffix.len())];
    format!("{}{suffix}", prefix.trim_end_matches('-'))
}

pub(crate) fn event_input(
    event: &Event,
    provider: &str,
    sequence: i64,
    received_at: &str,
) -> Value {
    let mut input = serde_json::to_value(event).expect("validated events serialize");
    input["provider"] = provider.into();
    input["sequence"] = sequence.into();
    input["received_at"] = received_at.into();
    input
}

pub(crate) fn matches(definition: &Definition, input: &Value) -> Result<bool, String> {
    let engine = engine();
    let ast = compile(&engine, &definition.script)?;
    let input = rhai::serde::to_dynamic(input).map_err(|error| error.to_string())?;
    engine
        .call_fn::<bool>(&mut rhai::Scope::new(), &ast, "matches", (input,))
        .map_err(|error| error.to_string())
}

pub(crate) fn render_prompt(template: &str, event: &Value) -> Result<String, String> {
    prompt::render(template, event)
}

#[cfg(test)]
#[path = "tests/rules.rs"]
mod tests;
