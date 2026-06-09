use std::time::Duration;

use serde_json::{json, Value};

use crate::{
    core::memory_manager_head::{
        fetch_memfs_role_memory_file, fetch_memfs_role_memory_status,
        fetch_memfs_role_memory_tree, search_memfs_role_memory, write_memfs_role_memory_entry,
        MemfsRoleMemoryFileResponse, MemfsRoleMemorySearchResponse,
        MemfsRoleMemoryStatusResponse, MemfsRoleMemoryTreeResponse,
        MemfsWriteRoleMemoryEntryRequest, MemfsWriteRoleMemoryEntryResponse,
    },
    errors::CustomError,
};

pub(crate) fn is_memfs_client_tool_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "memfs" | "memory_tree" | "memory_apply_patch" | "core_memory_append" | "core_memory_replace"
    ) || normalized.starts_with("memfs_") || normalized.starts_with("den_memfs")
}

pub(crate) fn native_runtime_memfs_unavailable_payload(tool_name: &str) -> Value {
    json!({
        "ok": false,
        "available": false,
        "storage": "sqlite",
        "tool": tool_name,
        "message": "MemFS tools are unavailable under AGENT_RUNTIME=native. Use Den memory tools (memory_write_entry, memory_browse, memory_read, memory_search, memory_status) instead.",
    })
}

pub(crate) fn filter_client_tools_for_native_runtime(client_tools: Option<&Value>) -> Option<Value> {
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

pub(crate) fn memfs_http_client(context: &str) -> Result<reqwest::Client, CustomError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| CustomError::System(format!("{context}: {e}")))
}

pub(crate) async fn fetch_role_memory_status(
    http: &reqwest::Client,
    base_url: &str,
    bear_id: uuid::Uuid,
    role: &str,
) -> Result<Option<MemfsRoleMemoryStatusResponse>, CustomError> {
    fetch_memfs_role_memory_status(http, base_url, bear_id, role).await
}

pub(crate) async fn fetch_role_memory_tree(
    http: &reqwest::Client,
    base_url: &str,
    bear_id: uuid::Uuid,
    role: &str,
) -> Result<Option<MemfsRoleMemoryTreeResponse>, CustomError> {
    fetch_memfs_role_memory_tree(http, base_url, bear_id, role).await
}

pub(crate) async fn fetch_role_memory_file(
    http: &reqwest::Client,
    base_url: &str,
    bear_id: uuid::Uuid,
    role: &str,
    path: &str,
) -> Result<Option<MemfsRoleMemoryFileResponse>, CustomError> {
    fetch_memfs_role_memory_file(http, base_url, bear_id, role, path).await
}

pub(crate) async fn search_role_memory(
    http: &reqwest::Client,
    base_url: &str,
    bear_id: uuid::Uuid,
    role: &str,
    query: &str,
    limit: Option<usize>,
) -> Result<Option<MemfsRoleMemorySearchResponse>, CustomError> {
    search_memfs_role_memory(http, base_url, bear_id, role, query, limit).await
}

pub(crate) async fn write_role_memory_entry(
    http: &reqwest::Client,
    base_url: &str,
    bear_id: uuid::Uuid,
    role: &str,
    request: &MemfsWriteRoleMemoryEntryRequest,
) -> Result<Option<MemfsWriteRoleMemoryEntryResponse>, CustomError> {
    write_memfs_role_memory_entry(http, base_url, bear_id, role, request).await
}
