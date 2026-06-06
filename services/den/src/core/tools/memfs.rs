use std::time::Duration;

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
