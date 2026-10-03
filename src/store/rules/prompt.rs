use std::sync::atomic::{AtomicUsize, Ordering};

use handlebars::{
    Context, Handlebars, Helper, HelperDef, HelperResult, Output, RenderContext, RenderErrorReason,
    Renderable, ScopedJson, StringOutput, Template,
    template::{HelperTemplate, Parameter, TemplateElement},
};
use serde_json::Value;

const MAX_BYTES: usize = 65_536;
const MAX_DEPTH: usize = 32;
const MAX_EVALUATIONS: usize = 50_000;

pub(super) fn compile(source: &str) -> Result<Template, String> {
    if source.len() > MAX_BYTES || source.trim().is_empty() || source.contains('\0') {
        return Err("prompt must contain text without NUL characters and fit within 64 KiB".into());
    }
    let mut template = Template::compile(source).map_err(|error| error.to_string())?;
    bound_template(&mut template, 0)?;
    Ok(template)
}

fn bound_template(template: &mut Template, depth: usize) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("prompt template exceeds 32 nesting levels".into());
    }
    for element in &mut template.elements {
        match element {
            TemplateElement::Expression(helper) | TemplateElement::HtmlExpression(helper)
                if helper.params.is_empty()
                    && helper.hash.is_empty()
                    && helper
                        .name
                        .as_name()
                        .is_some_and(|path| path == "event" || path.starts_with("event.")) =>
            {
                // Event-path interpolation uses JSON for collections and null, and literal strings.
                helper.params.push(helper.name.clone());
                helper.name = Parameter::Name("__tandem_value".into());
            }
            TemplateElement::HelperBlock(helper) => {
                for child in [&mut helper.template, &mut helper.inverse]
                    .into_iter()
                    .flatten()
                {
                    bound_template(child, depth + 1)?;
                }
            }
            TemplateElement::DecoratorBlock(partial) | TemplateElement::PartialBlock(partial) => {
                if let Some(child) = &mut partial.template {
                    bound_template(child, depth + 1)?;
                }
            }
            _ => {}
        }
    }
    let mut wrapper = HelperTemplate::new(
        handlebars::template::ExpressionSpecBuilder::default()
            .name(Parameter::Name("__tandem_prompt".into()))
            .params(Vec::new())
            .hash(Default::default())
            .omit_pre_ws(false)
            .omit_pro_ws(false)
            .build()
            .expect("complete prompt wrapper"),
        true,
        false,
    );
    wrapper.template = Some(std::mem::take(template));
    template
        .elements
        .push(TemplateElement::HelperBlock(Box::new(wrapper)));
    Ok(())
}

#[derive(Default)]
struct RenderBudget {
    evaluations: AtomicUsize,
    depth: AtomicUsize,
}

impl HelperDef for RenderBudget {
    fn call<'reg: 'rc, 'rc>(
        &self,
        helper: &Helper<'rc>,
        registry: &'reg Handlebars<'reg>,
        context: &'rc Context,
        render_context: &mut RenderContext<'reg, 'rc>,
        output: &mut dyn Output,
    ) -> HelperResult {
        if self.evaluations.fetch_add(1, Ordering::Relaxed) >= MAX_EVALUATIONS {
            return Err(RenderErrorReason::Other(
                "prompt exceeds 50000 template evaluations".into(),
            )
            .into());
        }
        if self.depth.load(Ordering::Relaxed) >= MAX_DEPTH {
            return Err(
                RenderErrorReason::Other("prompt exceeds 32 rendering levels".into()).into(),
            );
        }
        let template = helper
            .template()
            .ok_or_else(|| RenderErrorReason::Other("prompt budget requires a block".into()))?;
        self.depth.fetch_add(1, Ordering::Relaxed);
        let result = template.render(
            registry,
            context,
            render_context,
            &mut BoundedOutput { output, bytes: 0 },
        );
        self.depth.fetch_sub(1, Ordering::Relaxed);
        result
    }
}

struct BoundedOutput<'a> {
    output: &'a mut dyn Output,
    bytes: usize,
}

impl Output for BoundedOutput<'_> {
    fn write(&mut self, text: &str) -> std::io::Result<()> {
        if text.len() > MAX_BYTES - self.bytes {
            return Err(std::io::Error::other("resolved prompt exceeds 64 KiB"));
        }
        self.bytes += text.len();
        self.output.write(text)
    }
}

struct JsonValue;

impl HelperDef for JsonValue {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        helper: &Helper<'rc>,
        _: &'reg Handlebars<'reg>,
        _: &'rc Context,
        _: &mut RenderContext<'reg, 'rc>,
    ) -> Result<ScopedJson<'rc>, handlebars::RenderError> {
        let value = helper
            .param(0)
            .ok_or(RenderErrorReason::ParamNotFoundForIndex("json", 0))?;
        if value.is_value_missing() {
            return Err(handlebars::RenderError::strict_error(value.relative_path()));
        }
        let text = if helper.name() == "__tandem_value" {
            value
                .value()
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.value().to_string())
        } else {
            value.value().to_string()
        };
        Ok(ScopedJson::Derived(Value::String(text)))
    }
}

pub(super) fn render(source: &str, event: &Value) -> Result<String, String> {
    let template = compile(source)?;
    let mut registry = Handlebars::new();
    registry.set_strict_mode(true);
    registry.register_escape_fn(handlebars::no_escape);
    registry.unregister_helper("log");
    registry.register_helper("json", Box::new(JsonValue));
    registry.register_helper("__tandem_value", Box::new(JsonValue));
    registry.register_helper("__tandem_prompt", Box::<RenderBudget>::default());
    let context =
        Context::wraps(serde_json::json!({"event": event})).map_err(|error| error.to_string())?;
    let mut output = StringOutput::new();
    template
        .render(
            &registry,
            &context,
            &mut RenderContext::new(None),
            &mut output,
        )
        .map_err(|error| match error.reason() {
            RenderErrorReason::MissingVariable(Some(path)) => {
                format!("missing prompt field: {path}")
            }
            RenderErrorReason::MissingVariable(None) => "missing prompt field".into(),
            _ => error.to_string(),
        })?;
    let result = output.into_string().map_err(|error| error.to_string())?;
    if result.trim().is_empty() || result.contains('\0') {
        return Err("resolved prompt must contain text without NUL characters".into());
    }
    Ok(result)
}
