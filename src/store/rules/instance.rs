use super::{Definition, compile, engine};

#[derive(Debug, Default, PartialEq)]
pub(crate) struct InstanceOverrides {
    pub custom_name: Option<String>,
    pub custom_description: Option<String>,
}

pub(crate) fn resolve_instance_hooks(
    definition: &Definition,
    input: &serde_json::Value,
) -> Result<InstanceOverrides, String> {
    let ast = compile(&engine(), &definition.script)?;
    Ok(InstanceOverrides {
        custom_name: hook(&ast, "instance_name", input).and_then(|name| sanitize_name(&name)),
        custom_description: hook(&ast, "instance_description", input)
            .and_then(|description| validate_description(&description)),
    })
}

fn hook(ast: &rhai::AST, name: &str, input: &serde_json::Value) -> Option<String> {
    if !ast
        .iter_functions()
        .any(|function| function.name == name && function.params.len() == 1)
    {
        return None;
    }
    let input = rhai::serde::to_dynamic(input).ok()?;
    engine()
        .call_fn::<rhai::ImmutableString>(&mut rhai::Scope::new(), ast, name, (input,))
        .ok()
        .map(|value| value.to_string())
}

fn sanitize_name(source: &str) -> Option<String> {
    let mut name = String::new();
    for character in source.trim().chars() {
        if character.is_ascii_alphanumeric() {
            name.push(character.to_ascii_lowercase());
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
        if name.len() == 40 {
            break;
        }
    }
    let name = name.trim_end_matches('-').to_owned();
    crate::store::environments::validate_instance_name(&name)
        .ok()
        .map(|()| name)
}

fn validate_description(source: &str) -> Option<String> {
    let description = source.trim();
    (!description.is_empty()
        && description.len() <= 1000
        && !description.chars().any(char::is_control))
    .then(|| description.to_owned())
}

pub(crate) fn default_instance_description(rule: &str, summary: &str) -> String {
    format!("{rule}: {summary}")
}
