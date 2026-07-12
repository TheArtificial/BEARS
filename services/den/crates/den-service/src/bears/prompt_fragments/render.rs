use den_core::DenError;
use minijinja::context;
use minijinja::value::Value;
use minijinja::UndefinedBehavior;

use super::registry::PromptFragment;

#[derive(Debug, Clone)]
pub struct CompileTimePromptContext<'a> {
    pub bear_name: &'a str,
    pub bear_slug: &'a str,
}

pub fn render_compile_time_fragment(
    fragment: &PromptFragment,
    context: &CompileTimePromptContext<'_>,
) -> Result<String, DenError> {
    if fragment.frontmatter.templating_phase != "compile" {
        return Err(DenError::ValidationError(format!(
            "prompt fragment {} is not compile-time renderable",
            fragment.frontmatter.id
        )));
    }
    render_compile_time_text(&fragment.frontmatter.id, &fragment.body, context)
}

pub fn render_turn_fragment(
    fragment: &PromptFragment,
    context: &serde_json::Value,
) -> Result<String, DenError> {
    if fragment.frontmatter.templating_phase != "turn" {
        return Err(DenError::ValidationError(format!(
            "prompt fragment {} is not turn-time renderable",
            fragment.frontmatter.id
        )));
    }
    render_turn_text(&fragment.frontmatter.id, &fragment.body, context)
}

pub fn render_turn_text(
    source_label: &str,
    text: &str,
    context: &serde_json::Value,
) -> Result<String, DenError> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    let value = Value::from_serialize(context);
    env.render_str(text, value).map_err(|err| {
        DenError::Parsing(format!(
            "prompt text {source_label} failed to render: {err}"
        ))
    })
}

pub fn render_compile_time_text(
    source_label: &str,
    text: &str,
    context: &CompileTimePromptContext<'_>,
) -> Result<String, DenError> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.render_str(
        text,
        context! {
            bear_name => context.bear_name,
            bear_slug => context.bear_slug,
        },
    )
    .map_err(|err| {
        DenError::Parsing(format!(
            "prompt text {source_label} failed to render: {err}"
        ))
    })
}
