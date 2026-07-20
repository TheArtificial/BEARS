//! Native-runtime guards that reject removed legacy memory client tools.

use std::borrow::Cow;

use serde_json::Value;

pub fn is_legacy_memory_client_tool_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "memory_tree" | "memory_apply_patch" | "core_memory_append" | "core_memory_replace"
    )
}

pub fn filter_client_tools_for_native_runtime(
    client_tools: Option<&Value>,
) -> Option<Cow<'_, Value>> {
    let value = client_tools?;
    let Some(items) = value.as_array() else {
        return Some(Cow::Borrowed(value));
    };
    let filtered: Vec<Value> = items
        .iter()
        .filter(|item| {
            item.get("name")
                .and_then(|v| v.as_str())
                .map(|name| !is_legacy_memory_client_tool_name(name))
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    Some(Cow::Owned(Value::Array(filtered)))
}
