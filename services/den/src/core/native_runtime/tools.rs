use serde_json::Value;

use crate::{
    core::{
        bears::BearAgentRole,
        llm::LlmToolDefinition,
        tools::descriptor::builtin_den_tool_descriptors_for_role,
    },
    errors::CustomError,
};

pub fn den_tools_for_role(role: BearAgentRole) -> Vec<LlmToolDefinition> {
    builtin_den_tool_descriptors_for_role(role)
        .into_iter()
        .map(|d| LlmToolDefinition {
            name: d.provider_name.to_string(),
            description: Some(d.description.to_string()),
            parameters: d.input_schema.clone(),
        })
        .collect()
}

pub fn merge_den_and_client_tools(
    role: BearAgentRole,
    client_tools: Option<&Value>,
) -> Result<Vec<LlmToolDefinition>, CustomError> {
    let mut merged = den_tools_for_role(role);
    let Some(client_tools) = client_tools.and_then(|v| v.as_array()) else {
        return Ok(merged);
    };
    let mut seen = std::collections::HashSet::<String>::new();
    for tool in &merged {
        seen.insert(tool.name.clone());
    }
    for item in client_tools {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(name) = name else {
            continue;
        };
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
    Ok(merged)
}
