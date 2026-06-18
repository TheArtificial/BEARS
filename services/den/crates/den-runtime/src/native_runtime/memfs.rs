//! Native-runtime guards that reject legacy MemFS client tools. The MemFS HTTP
//! transport itself is gone; these helpers keep legacy `memfs*` tool calls from
//! reaching the Den-native runtime.

use serde_json::Value;

pub fn is_memfs_client_tool_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "memfs" | "memory_tree" | "memory_apply_patch" | "core_memory_append" | "core_memory_replace"
    ) || normalized.starts_with("memfs_") || normalized.starts_with("den_memfs")
}

pub fn filter_client_tools_for_native_runtime(client_tools: Option<&Value>) -> Option<Value> {
    let Some(items) = client_tools.and_then(|v| v.as_array()) else {
        return client_tools.cloned();
    };
    let filtered: Vec<Value> = items
        .iter()
        .filter(|item| {
            item.get("name")
                .and_then(|v| v.as_str())
                .map(|name| !is_memfs_client_tool_name(name))
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    Some(Value::Array(filtered))
}
