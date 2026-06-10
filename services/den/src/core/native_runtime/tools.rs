use serde_json::Value;

use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        llm::LlmToolDefinition,
        tools::{
            descriptor::builtin_den_tool_descriptors_for_role,
            memfs::{filter_client_tools_for_native_runtime, is_memfs_client_tool_name},
        },
    },
    errors::CustomError,
};

pub fn den_tools_for_role(role: BearProfile) -> Vec<LlmToolDefinition> {
    builtin_den_tool_descriptors_for_role(role)
        .into_iter()
        .map(|d| LlmToolDefinition {
            name: d.provider_name.to_string(),
            description: Some(d.description.to_string()),
            parameters: d.input_schema.clone(),
        })
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

pub fn merge_den_and_client_tools(
    config: &Config,
    role: BearProfile,
    client_tools: Option<&Value>,
) -> Result<Vec<LlmToolDefinition>, CustomError> {
    let mut merged = den_tools_for_role(role);
    let filtered_client_tools = if config.uses_native_agent_runtime() {
        filter_client_tools_for_native_runtime(client_tools)
    } else {
        client_tools.cloned()
    };
    let Some(client_tools) = filtered_client_tools.as_ref().and_then(|v| v.as_array()) else {
        return Ok(merged);
    };
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
            description: item
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string),
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
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn mcp_dedup_key_uses_action_suffix() {
        assert_eq!(
            mcp_client_tool_dedup_key("mcp__chrome_devtools_mcp_zed__click"),
            Some("click")
        );
    }

    #[test]
    fn merge_skips_duplicate_mcp_action_suffixes() {
        let config = Config::test_stub();
        let client_tools = serde_json::json!([
            {"name": "mcp__chrome_devtools_mcp_zed__click", "parameters": {"type": "object"}},
            {"name": "mcp__chrome_devtools_custom__click", "parameters": {"type": "object"}},
            {"name": "fs_read_text_file", "parameters": {"type": "object"}},
        ]);
        let merged =
            merge_den_and_client_tools(&config, BearProfile::Pair, Some(&client_tools)).unwrap();
        let names: Vec<_> = merged.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"mcp__chrome_devtools_mcp_zed__click"));
        assert!(!names.contains(&"mcp__chrome_devtools_custom__click"));
        assert!(names.contains(&"fs_read_text_file"));
    }
}
