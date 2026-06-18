use crate::core::tools::{
    aliases::is_builtin_den_tool,
    constants::*,
    descriptor::{builtin_den_tool_descriptors, builtin_den_tool_descriptors_for_profile},
};
use den_runtime::bears::BearProfile;
use std::collections::HashSet;

#[test]
fn provider_names_are_safe_and_unique() {
    let descriptors = builtin_den_tool_descriptors();
    let mut provider_names = HashSet::new();
    for descriptor in descriptors {
        assert!(
            descriptor
                .provider_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "provider name must be Letta/provider-safe: {}",
            descriptor.provider_name
        );
        assert!(!descriptor.provider_name.contains('.'));
        assert!(!descriptor.provider_name.contains('/'));
        assert!(
            provider_names.insert(descriptor.provider_name.clone()),
            "duplicate provider name: {}",
            descriptor.provider_name
        );
    }
}

#[test]
fn canonical_dotted_names_map_to_provider_safe_aliases() {
    let descriptors = builtin_den_tool_descriptors();
    let task = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_TASK_WRITE_INTENT)
        .expect("task intent descriptor exists");
    assert_eq!(task.provider_name, "den_task_write_intent");

    let skill = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_SKILL_PROPOSE)
        .expect("skill proposal descriptor exists");
    assert_eq!(skill.provider_name, "den_skill_propose");

    let conversation_title = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_CONVERSATION_SET_TITLE)
        .expect("conversation title descriptor exists");
    assert_eq!(
        conversation_title.provider_name,
        DEN_CONVERSATION_SET_TITLE_PROVIDER
    );

    let web_fetch = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_WEB_FETCH)
        .expect("web fetch descriptor exists");
    assert_eq!(web_fetch.provider_name, DEN_WEB_FETCH_PROVIDER);

    let web_search = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_WEB_SEARCH)
        .expect("web search descriptor exists");
    assert_eq!(web_search.provider_name, DEN_WEB_SEARCH_PROVIDER);

    let bear_environment = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_BEAR_ENVIRONMENT)
        .expect("bear environment descriptor exists");
    assert_eq!(
        bear_environment.provider_name,
        DEN_BEAR_ENVIRONMENT_PROVIDER
    );

    let situation = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_SITUATION_GET)
        .expect("situation descriptor exists");
    assert_eq!(situation.provider_name, DEN_SITUATION_GET_PROVIDER);
    assert_eq!(situation.provider_name, "session_info");
    assert_ne!(situation.provider_name, "situation_get");
    assert_ne!(situation.provider_name, "den_situation_get");

    let memory_browse = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_MEMORY_TREE)
        .expect("memory browse descriptor exists");
    assert_eq!(memory_browse.provider_name, DEN_MEMORY_TREE_PROVIDER);
    assert_eq!(memory_browse.provider_name, "memory_browse");
    assert_ne!(memory_browse.provider_name, "memory_tree");

    let memory = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_MEMORY_WRITE_ENTRY)
        .expect("memory write descriptor exists");
    assert_eq!(memory.provider_name, DEN_MEMORY_WRITE_ENTRY_PROVIDER);

    let update_plan = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_WORK_PLAN_UPDATE)
        .expect("work plan update descriptor exists");
    assert_eq!(update_plan.provider_name, DEN_WORK_PLAN_UPDATE_PROVIDER);
    assert_eq!(update_plan.provider_name, "update_plan");

    let enter_plan_mode = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_PLAN_MODE_ENTER)
        .expect("enter plan mode descriptor exists");
    assert_eq!(enter_plan_mode.provider_name, DEN_PLAN_MODE_ENTER_PROVIDER);
    assert_eq!(enter_plan_mode.provider_name, "enter_plan_mode");
}

#[test]
fn den_server_tools_advertise_semantic_aliases_not_legacy_den_prefixes() {
    let provider_names = builtin_den_tool_descriptors_for_profile(BearProfile::Pair)
        .into_iter()
        .map(|descriptor| descriptor.provider_name)
        .collect::<HashSet<_>>();
    assert!(provider_names.contains("session_info"));
    assert!(provider_names.contains("bear_environment"));
    assert!(provider_names.contains("set_conversation_title"));
    assert!(provider_names.contains("web_search"));
    assert!(provider_names.contains("memory_browse"));
    assert!(provider_names.contains("memory_read"));
    assert!(provider_names.contains("update_plan"));
    assert!(provider_names.contains("enter_plan_mode"));
    assert!(provider_names.contains("record_plan_approval"));
    assert!(provider_names.contains("exit_plan_mode"));
    assert!(provider_names.contains("cancel_plan_mode"));
    assert!(!provider_names.contains("situation_get"));
    assert!(!provider_names.contains("memory_tree"));
    assert!(!provider_names.contains("den_situation_get"));
    assert!(!provider_names.contains("den_web_search"));
    assert!(!provider_names.contains("den_memory_read"));
    assert!(!provider_names.contains("den_work_plan_update"));
    assert!(!provider_names.contains("den_plan_mode_enter"));
}

#[test]
fn all_descriptors_are_known_tools() {
    for descriptor in builtin_den_tool_descriptors() {
        assert!(
            is_builtin_den_tool(descriptor.name),
            "unknown descriptor name: {}",
            descriptor.name
        );
    }
}

#[test]
fn den_tool_display_json_includes_memory_titles() {
    use crate::core::tools::descriptor::{
        den_tool_display_json_for_provider, den_tool_policy_json_for_provider,
    };

    let read = den_tool_display_json_for_provider(
        "memory_read",
        &serde_json::json!({ "path": "pair/notes/example.md" }),
    )
    .expect("memory_read display");
    assert_eq!(read["title"], "Reading memory pair/notes/example.md");
    assert_eq!(read["progress"], "Reading memory");

    let write = den_tool_display_json_for_provider(
        "memory_write_entry",
        &serde_json::json!({ "title": "Saved fact", "path": "pair/notes/mem_1.md" }),
    )
    .expect("memory_write_entry display");
    assert!(write["title"]
        .as_str()
        .unwrap_or("")
        .starts_with("Writing memory entry"));
    assert_eq!(write["progress"], "Writing memory entry");

    let policy = den_tool_policy_json_for_provider("memory_read").expect("memory_read policy");
    assert_eq!(policy["execution_target"], "den");
}
