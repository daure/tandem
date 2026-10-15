use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Acceptance, Definition, prompt, validate_prompt};
use crate::store::{environments, opencode::Launch};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ResolvedSettings {
    pub template: String,
    pub target_instance: Option<String>,
    pub model: String,
    pub variant: Option<String>,
}

pub(crate) fn is_template(value: &str) -> bool {
    value.contains("{{")
}

pub(super) fn validate_sources(definition: &Definition) -> Result<(), String> {
    for (label, value) in [
        ("handler template", Some(definition.template.as_str())),
        ("target instance", definition.target_instance.as_deref()),
        ("session model", Some(definition.model.as_str())),
        ("thinking variant", definition.variant.as_deref()),
    ] {
        if let Some(value) = value {
            validate_prompt(value).map_err(|error| format!("{label}: {error}"))?;
        }
    }
    if !is_template(&definition.template) {
        environments::validate_name(&definition.template)?;
    }
    if let Some(target) = &definition.target_instance
        && !is_template(target)
    {
        environments::validate_instance_name(target)?;
    }
    Launch {
        model: (!is_template(&definition.model)).then(|| definition.model.clone()),
        variant: definition
            .variant
            .clone()
            .filter(|value| !is_template(value)),
        prompt: None,
    }
    .validate()
}

impl Definition {
    pub(crate) fn resolve_settings(&self, event: &Value) -> Result<ResolvedSettings, String> {
        let render = |label: &str, value: &str| {
            prompt::render_value(value, event).map_err(|error| format!("{label}: {error}"))
        };
        let settings = ResolvedSettings {
            template: render("handler template", &self.template)?,
            target_instance: self
                .target_instance
                .as_deref()
                .map(|value| render("target instance", value))
                .transpose()?,
            model: render("session model", &self.model)?,
            variant: self
                .variant
                .as_deref()
                .map(|value| render("thinking variant", value))
                .transpose()?
                .filter(|value| !value.is_empty()),
        };
        environments::validate_name(&settings.template)
            .map_err(|error| format!("handler template: {error}"))?;
        if let Some(target) = &settings.target_instance {
            environments::validate_instance_name(target)
                .map_err(|error| format!("target instance: {error}"))?;
        }
        Launch {
            model: Some(settings.model.clone()),
            variant: settings.variant.clone(),
            prompt: None,
        }
        .validate()
        .map_err(|error| format!("session model/thinking variant: {error}"))?;
        Ok(settings)
    }
}

impl Acceptance {
    pub(crate) fn template(&self) -> &str {
        self.resolved_settings
            .as_ref()
            .map_or(self.rule.definition.template.as_str(), |settings| {
                &settings.template
            })
    }

    pub(crate) fn launch_definition(&self) -> Definition {
        let mut definition = self.rule.definition.clone();
        if let Some(settings) = &self.resolved_settings {
            definition.template.clone_from(&settings.template);
            definition
                .target_instance
                .clone_from(&settings.target_instance);
            definition.model.clone_from(&settings.model);
            definition.variant.clone_from(&settings.variant);
        }
        definition
    }

    pub(crate) fn session_prompt(&self, creating: bool) -> Result<&str, String> {
        let prompt = if !creating && self.rule.definition.follow_up_prompt.is_some() {
            self.resolved_follow_up_prompt.as_deref()
        } else {
            self.resolved_prompt.as_deref()
        };
        prompt.ok_or_else(|| "resolved prompt unavailable".into())
    }
}
