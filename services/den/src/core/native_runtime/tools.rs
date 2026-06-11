use serde_json::Value;

use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        llm::LlmToolDefinition,
        tools::{
            descriptor::{
                builtin_den_tool_descriptors_for_pair_acp_surface,
                builtin_den_tool_descriptors_for_profile, DenToolDescriptor,
            },
            memfs::{filter_client_tools_for_native_runtime, is_memfs_client_tool_name},
        },
    },
    errors::CustomError,
};

/// Pair turns omit adapter workspace/MCP tools unless the prompt suggests repo/file work.
pub fn pair_turn_needs_workspace_client_tools(prompt: Option<&str>) -> bool {
    let Some(prompt) = prompt.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let lower = prompt.to_ascii_lowercase();
    const KEYWORDS: &[&str] = &[
        "file",
        "edit",
        "refactor",
        "implement",
        "fix ",
        "code",
        "function",
        "class",
        "test ",
        "compile",
        "build ",
        "terminal",
        "grep",
        "codebase",
        "in the repo",
        "workspace",
        "create a ",
        "add a ",
        "delete ",
        "rename ",
        "move ",
        "patch",
        "diff",
        "git ",
        "cargo ",
        "npm ",
        "docker",
        "directory",
        "folder",
        "read ",
        "write ",
        "search replace",
    ];
    KEYWORDS.iter().any(|keyword| lower.contains(keyword))
}

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

pub fn den_tools_for_profile(config: &Config, role: BearProfile) -> Vec<LlmToolDefinition> {
    let compact = config.uses_native_agent_runtime();
    let descriptors = if config.uses_native_agent_runtime() && role == BearProfile::Pair {
        builtin_den_tool_descriptors_for_pair_acp_surface()
    } else {
        builtin_den_tool_descriptors_for_profile(role)
    };
    descriptors
        .into_iter()
        .map(|descriptor| den_tool_to_llm_definition(&descriptor, compact))
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

pub fn merge_den_and_client_tools(
    config: &Config,
    role: BearProfile,
    client_tools: Option<&Value>,
    pair_turn_prompt: Option<&str>,
) -> Result<Vec<LlmToolDefinition>, CustomError> {
    let mut merged = den_tools_for_profile(config, role);
    let include_client_tools = if config.uses_native_agent_runtime() && role == BearProfile::Pair {
        pair_turn_needs_workspace_client_tools(pair_turn_prompt)
    } else {
        true
    };
    if !include_client_tools {
        tracing::info!(
            role = %role.as_str(),
            den_tool_count = merged.len(),
            "native pair turn using Den-only tool surface (workspace client tools deferred)"
        );
        return Ok(merged);
    }
    let filtered_client_tools = if config.uses_native_agent_runtime() {
        filter_client_tools_for_native_runtime(client_tools)
    } else {
        client_tools.cloned()
    };
    let Some(client_tools) = filtered_client_tools.as_ref().and_then(|v| v.as_array()) else {
        return Ok(merged);
    };
    let compact = config.uses_native_agent_runtime();
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
        if config.uses_native_agent_runtime() && is_memfs_client_tool_name(name) {
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
    use crate::config::{AgentRuntimeMode, Config};

    fn native_test_config() -> Config {
        let mut config = Config::test_stub();
        config.agent_runtime_mode = AgentRuntimeMode::Native;
        config
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
            Some("edit the file main.rs"),
        )
        .unwrap();
        let names: Vec<_> = merged.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"mcp__chrome_devtools_mcp_zed__click"));
        assert!(!names.contains(&"mcp__chrome_devtools_custom__click"));
        assert!(names.contains(&"fs_read_text_file"));
    }

    #[test]
    fn pair_memory_question_omits_workspace_client_tools() {
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
        assert!(!names.contains(&"fs_read_text_file"));
        assert!(!names.iter().any(|name| name.starts_with("mcp__")));
        assert!(merged.len() <= 20);
    }

    #[test]
    fn pair_workspace_prompt_includes_client_tools() {
        let config = native_test_config();
        let client_tools = serde_json::json!([
            {"name": "fs_read_text_file", "parameters": {"type": "object"}},
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
    }
}
