use den_core::{DenError, config::Config};
use serde_json::Value;

use den_service::bears::BearProfile;
use crate::llm::LlmToolDefinition;
use den_core::tools::descriptor::{
    DenToolDescriptor, builtin_den_tool_descriptors_for_pair_acp_surface,
    builtin_den_tool_descriptors_for_profile,
};

use super::legacy_memory_tools::{
    filter_client_tools_for_native_runtime, is_legacy_memory_client_tool_name,
};

fn den_tool_to_llm_definition(descriptor: &DenToolDescriptor, compact: bool) -> LlmToolDefinition {
    LlmToolDefinition {
        name: descriptor.provider_name.clone(),
        description: Some(if compact {
            descriptor.label.to_string()
        } else {
            descriptor.description.to_string()
        }),
        parameters: descriptor.input_schema.clone(),
    }
}

pub fn den_tools_for_profile(_config: &Config, role: BearProfile) -> Vec<LlmToolDefinition> {
    let descriptors = if role == BearProfile::Pair {
        builtin_den_tool_descriptors_for_pair_acp_surface()
    } else {
        builtin_den_tool_descriptors_for_profile(role)
    };
    descriptors
        .into_iter()
        .map(|descriptor| den_tool_to_llm_definition(&descriptor, true))
        .collect()
}

/// Collapse duplicate forwarded MCP tools that share the same action suffix, e.g.
/// `mcp__chrome_devtools_mcp_zed__click` and `mcp__chrome_devtools_custom__click`.
fn mcp_client_tool_dedup_key(name: &str) -> Option<&str> {
    if !name.starts_with("mcp__") {
        return None;
    }
    name.rsplit_once("__").map(|(_, action)| action)
}

fn compact_client_tool_description(description: Option<&str>) -> Option<String> {
    let description = description?.trim();
    if description.is_empty() {
        return None;
    }
    let first_sentence = description
        .split_once(". ")
        .map(|(head, _)| head)
        .unwrap_or(description);
    let compact = if first_sentence.len() > 96 {
        format!("{}…", &first_sentence[..96])
    } else {
        first_sentence.to_string()
    };
    Some(compact)
}

/// Browser chat turns omit the full Den tool surface unless the prompt suggests tool-relevant work.
pub fn chat_turn_needs_full_tool_surface(prompt: Option<&str>) -> bool {
    let Some(prompt) = prompt.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    if chat_turn_is_capabilities_meta_query(prompt) {
        return false;
    }
    let lower = prompt.to_ascii_lowercase();
    const KEYWORDS: &[&str] = &[
        "memory",
        "remember",
        "recall",
        "search",
        "browse",
        "work plan",
        "workboard",
        "handoff",
        "fetch ",
        "http",
        "https://",
        "url ",
        "web ",
        "title",
        "rename conversation",
        "policy",
        "members",
        "proposal",
        "review",
        "curate",
        "write ",
        "save ",
        "update ",
        "plan mode",
        "file",
        "code",
        "workspace",
    ];
    KEYWORDS.iter().any(|keyword| lower.contains(keyword))
}

pub fn chat_turn_is_capabilities_meta_query(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    const PHRASES: &[&str] = &[
        "list capabilities",
        "list your capabilities",
        "list tools",
        "list your tools",
        "what tools",
        "what capabilities",
        "which tools",
        "which capabilities",
        "what can you do",
        "what do you have access",
        "show me your tools",
        "show your tools",
        "available tools",
        "available capabilities",
        "tools do you have",
        "capabilities do you have",
    ];
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

pub fn merge_den_and_client_tools(
    config: &Config,
    role: BearProfile,
    client_tools: Option<&Value>,
    pair_turn_prompt: Option<&str>,
) -> Result<Vec<LlmToolDefinition>, DenError> {
    let mut merged = if role == BearProfile::Chat
        && !chat_turn_needs_full_tool_surface(pair_turn_prompt)
    {
        tracing::info!(
            role = %role.as_str(),
            "native chat turn using empty tool surface (informational prompt; tool list is in system context)"
        );
        Vec::new()
    } else {
        den_tools_for_profile(config, role)
    };
    if role == BearProfile::Chat {
        return Ok(merged);
    }
    let filtered_client_tools = filter_client_tools_for_native_runtime(client_tools);
    let Some(client_tools) = filtered_client_tools.as_ref().and_then(|v| v.as_array()) else {
        return Ok(merged);
    };
    let compact = true;
    let mut seen = std::collections::HashSet::<String>::new();
    let mut seen_mcp_actions = std::collections::HashSet::<String>::new();
    for tool in &merged {
        seen.insert(tool.name.clone());
    }
    let mut skipped_mcp_duplicates = 0usize;
    for item in client_tools {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(name) = name else {
            continue;
        };
        if is_legacy_memory_client_tool_name(name) {
            continue;
        }
        if let Some(action) = mcp_client_tool_dedup_key(name) {
            if !seen_mcp_actions.insert(action.to_string()) {
                skipped_mcp_duplicates += 1;
                continue;
            }
        }
        if !seen.insert(name.to_string()) {
            continue;
        }
        merged.push(LlmToolDefinition {
            name: name.to_string(),
            description: if compact {
                compact_client_tool_description(item.get("description").and_then(|v| v.as_str()))
            } else {
                item.get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            },
            parameters: item
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
        });
    }
    if skipped_mcp_duplicates > 0 {
        tracing::info!(
            skipped_mcp_duplicates,
            merged_tool_count = merged.len(),
            "deduplicated forwarded MCP client tools with identical action suffixes"
        );
    }
    tracing::info!(
        role = %role.as_str(),
        den_tool_count = merged.len(),
        client_tool_count = client_tools.len(),
        "merged native turn tool surface"
    );
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use den_core::config::Config;

    fn native_test_config() -> Config {
        Config::test_stub()
    }

    #[test]
    fn mcp_dedup_key_uses_action_suffix() {
        assert_eq!(
            mcp_client_tool_dedup_key("mcp__chrome_devtools_mcp_zed__click"),
            Some("click")
        );
    }

    #[test]
    fn merge_skips_duplicate_mcp_action_suffixes() {
        let config = native_test_config();
        let client_tools = serde_json::json!([
            {"name": "mcp__chrome_devtools_mcp_zed__click", "parameters": {"type": "object"}},
            {"name": "mcp__chrome_devtools_custom__click", "parameters": {"type": "object"}},
            {"name": "fs_read_text_file", "parameters": {"type": "object"}},
        ]);
        let merged = merge_den_and_client_tools(
            &config,
            BearProfile::Pair,
            Some(&client_tools),
            Some("click the browser page button"),
        )
        .unwrap();
        let names: Vec<_> = merged.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"mcp__chrome_devtools_mcp_zed__click"));
        assert!(!names.contains(&"mcp__chrome_devtools_custom__click"));
        assert!(names.contains(&"fs_read_text_file"));
        assert!(names.contains(&"session_info"));
    }

    #[test]
    fn pair_memory_question_keeps_stable_den_and_client_tool_surface() {
        let config = native_test_config();
        let client_tools = serde_json::json!([
            {"name": "fs_read_text_file", "parameters": {"type": "object"}},
            {"name": "mcp__chrome_devtools_mcp_zed__click", "parameters": {"type": "object"}},
        ]);
        let merged = merge_den_and_client_tools(
            &config,
            BearProfile::Pair,
            Some(&client_tools),
            Some("what do you know about me?"),
        )
        .unwrap();
        let names: Vec<_> = merged.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"session_info"));
        assert!(names.contains(&"fs_read_text_file"));
        assert!(names.iter().any(|name| name.starts_with("mcp__")));
    }

    #[test]
    fn pair_read_prompt_keeps_stable_den_and_client_tool_surface() {
        let config = native_test_config();
        let client_tools = serde_json::json!([
            {"name": "fs_read_text_file", "parameters": {"type": "object"}},
            {"name": "fs_find_paths", "parameters": {"type": "object"}},
            {"name": "fs_edit_file", "parameters": {"type": "object"}},
            {"name": "terminal_run_command", "parameters": {"type": "object"}},
            {"name": "mcp__chrome_devtools_mcp_zed__click", "parameters": {"type": "object"}}
        ]);
        let merged = merge_den_and_client_tools(
            &config,
            BearProfile::Pair,
            Some(&client_tools),
            Some("please read README.md"),
        )
        .unwrap();
        let names: Vec<_> = merged.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"fs_read_text_file"));
        assert!(names.contains(&"fs_find_paths"));
        assert!(names.contains(&"fs_edit_file"));
        assert!(names.contains(&"terminal_run_command"));
        assert!(names.iter().any(|name| name.starts_with("mcp__")));
        assert!(names.contains(&"session_info"));
    }

    #[test]
    fn pair_workspace_edit_prompt_includes_write_client_tools() {
        let config = native_test_config();
        let client_tools = serde_json::json!([
            {"name": "fs_read_text_file", "parameters": {"type": "object"}},
            {"name": "fs_edit_file", "parameters": {"type": "object"}},
        ]);
        let merged = merge_den_and_client_tools(
            &config,
            BearProfile::Pair,
            Some(&client_tools),
            Some("please edit the file src/lib.rs"),
        )
        .unwrap();
        let names: Vec<_> = merged.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"fs_read_text_file"));
        assert!(names.contains(&"fs_edit_file"));
        assert!(names.contains(&"session_info"));
    }

    #[test]
    fn pair_workspace_build_prompt_includes_terminal_client_and_den_tools() {
        let config = native_test_config();
        let client_tools = serde_json::json!([
            {"name": "fs_read_text_file", "parameters": {"type": "object"}},
            {"name": "terminal_run_command", "parameters": {"type": "object"}},
            {"name": "process_run", "parameters": {"type": "object"}},
            {"name": "mcp__chrome_devtools_mcp_zed__click", "parameters": {"type": "object"}}
        ]);
        let merged = merge_den_and_client_tools(
            &config,
            BearProfile::Pair,
            Some(&client_tools),
            Some("please build the project and inspect errors"),
        )
        .unwrap();
        let names: Vec<_> = merged.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"terminal_run_command"));
        assert!(names.contains(&"process_run"));
        assert!(names.contains(&"fs_read_text_file"));
        assert!(names.contains(&"session_info"));
        assert!(names.iter().any(|name| name.starts_with("mcp__")));
    }

    #[test]
    fn curate_profile_includes_proposal_and_core_tools() {
        let config = native_test_config();
        let merged = merge_den_and_client_tools(&config, BearProfile::Curate, None, None).unwrap();
        let names: Vec<_> = merged.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"memory_list_proposals"));
        assert!(names.contains(&"memory_read_proposal"));
        assert!(names.contains(&"memory_apply_core_update"));
        assert!(names.contains(&"memory_read"));
        assert!(!names.contains(&"enter_plan_mode"));
    }

    #[test]
    fn chat_capabilities_query_omits_den_tools() {
        let config = native_test_config();
        let merged = merge_den_and_client_tools(
            &config,
            BearProfile::Chat,
            None,
            Some("list your capabilities"),
        )
        .unwrap();
        assert!(merged.is_empty());
    }

    #[test]
    fn chat_memory_prompt_includes_memory_tools() {
        let config = native_test_config();
        let merged = merge_den_and_client_tools(
            &config,
            BearProfile::Chat,
            None,
            Some("search memory for deployment notes"),
        )
        .unwrap();
        let names: Vec<_> = merged.iter().map(|tool| tool.name.as_str()).collect();
        assert!(names.contains(&"memory_search"));
        assert!(names.contains(&"session_info"));
    }

    #[test]
    fn chat_memory_prompt_includes_den_tools() {
        let config = native_test_config();
        let merged = merge_den_and_client_tools(
            &config,
            BearProfile::Chat,
            None,
            Some("search memory for deployment notes"),
        )
        .unwrap();
        assert!(!merged.is_empty());
    }

    #[test]
    fn chat_turn_is_capabilities_meta_query_matches_common_phrases() {
        assert!(chat_turn_is_capabilities_meta_query("list your tools"));
        assert!(chat_turn_is_capabilities_meta_query(
            "What capabilities do you have?"
        ));
        assert!(!chat_turn_is_capabilities_meta_query(
            "search memory for onboarding"
        ));
    }
}
