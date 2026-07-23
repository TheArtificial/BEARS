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
    assert!(fragment
        .body
        .contains("You are operating as a Bear in Den."));
}

#[test]
fn repository_registry_contains_work_stance_fragment() {
    let registry = repository_prompt_fragment_registry().unwrap();
    let fragment = registry.require("stance_work").unwrap();
    assert_eq!(fragment.frontmatter.templating_phase, "compile");
    assert!(fragment.body.contains("Execution Space"));
}

#[test]
fn repository_bundle_references_pair_stance_fragment() {
    let fragments = repository_prompt_fragment_registry().unwrap();
    let bundles = repository_prompt_bundle_registry(&fragments).unwrap();
    let bundle = bundles.require("pair").unwrap();
    assert_eq!(bundle.fragments, vec!["den_baseline", "stance_pair"]);
}

#[test]
fn bundle_validation_rejects_missing_fragment() {
    let fragments = repository_prompt_fragment_registry().unwrap();
    let err = PromptBundleRegistry::from_embedded_sources(
        &[("bad.yaml", "id: bad\nfragments:\n  - missing_fragment\n")],
        &fragments,
    )
    .unwrap_err();
    assert!(err.to_string().contains("missing_fragment"));
}

#[test]
fn renders_repository_pair_bundle_fragments() {
    let fragments = repository_prompt_fragment_registry().unwrap();
    let bundles = repository_prompt_bundle_registry(&fragments).unwrap();
    let rendered = render_compile_time_bundle_fragments(
        bundles.require("pair").unwrap(),
        &fragments,
        &CompileTimePromptContext {
            bear_name: "Builder Bear",
            bear_slug: "builder",
        },
    )
    .unwrap();
    assert_eq!(rendered.len(), 2);
    assert_eq!(rendered[0].id, "den_baseline");
    assert_eq!(rendered[1].id, "stance_pair");
    assert!(rendered[1].body.contains("You are Builder Bear"));
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
fn focused_runtime_fragments_keep_execution_moving_across_tasks() {
    let registry = repository_prompt_fragment_registry().unwrap();

    let focused = registry.require("runtime_objective_focused").unwrap();
    assert!(focused
        .body
        .contains("active objective across task boundaries"));
    assert!(focused
        .body
        .contains("continue the next incomplete unblocked task"));

    let execution = registry.require("runtime_docket_execution_active").unwrap();
    assert!(execution.body.contains("satisfy the Job's commit policy"));
    assert!(execution
        .body
        .contains("refresh the focused Job/task state"));
    assert!(execution
        .body
        .contains("This task was explicitly retried after a blocked attempt"));
    assert!(execution
        .body
        .contains("check whether the prior blocker still applies"));
    assert!(execution
        .body
        .contains("continue the next incomplete unblocked task"));
    assert!(execution
        .body
        .contains("Final-answer only when the Job is complete"));
}

#[test]
fn pair_fragment_treats_jobs_as_the_dispatch_unit() {
    let registry = repository_prompt_fragment_registry().unwrap();
    let guidance = registry.require("stance_job_dispatch").unwrap();
    assert!(guidance.body.contains("Docket Job is the complete unit"));
    assert!(guidance
        .body
        .contains("call `dispatch_work` once with the `job_id`"));
    assert!(guidance
        .body
        .contains("executes their runs sequentially in task order"));
}

#[test]
fn repository_source_version_is_stable() {
    assert_eq!(
        repository_prompt_source_version(),
        repository_prompt_source_version()
    );
}
