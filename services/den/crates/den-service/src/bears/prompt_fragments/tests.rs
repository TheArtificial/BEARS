use super::*;
use super::{frontmatter::split_frontmatter, render::CompileTimePromptContext};

#[test]
fn splits_frontmatter_from_markdown_body() {
    let source = "---\nid: demo\nlayer: base\ntemplating_phase: compile\n---\n\nHello";
    let (yaml, body) = split_frontmatter("demo.md", source).unwrap();
    assert!(yaml.contains("id: demo"));
    assert_eq!(body.trim(), "Hello");
}

#[test]
fn repository_registry_contains_den_baseline() {
    let registry = repository_prompt_fragment_registry().unwrap();
    let fragment = registry.require("den_baseline").unwrap();
    assert_eq!(fragment.frontmatter.templating_phase, "compile");
    assert!(fragment.body.contains("You are operating as a Bear in Den."));
}

#[test]
fn renders_compile_time_fragment_with_allowed_variables() {
    let registry = PromptFragmentRegistry::from_embedded_sources(&[(
        "demo.md",
        "---\nid: demo\nlayer: base\ntemplating_phase: compile\nvars: [bear_name]\n---\nHello {{ bear_name }}",
    )])
    .unwrap();
    let rendered = render_compile_time_fragment(
        registry.require("demo").unwrap(),
        &CompileTimePromptContext {
            bear_name: "Builder Bear",
            bear_slug: "builder",
        },
    )
    .unwrap();
    assert_eq!(rendered, "Hello Builder Bear");
}

#[test]
fn rejects_unknown_turn_time_variables_in_compile_time_text() {
    let err = render_compile_time_text(
        "demo",
        "Today is {{ current_date }}",
        &CompileTimePromptContext {
            bear_name: "Builder Bear",
            bear_slug: "builder",
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("failed to render"));
}

#[test]
fn repository_source_version_is_stable() {
    assert_eq!(
        repository_prompt_source_version(),
        repository_prompt_source_version()
    );
}
