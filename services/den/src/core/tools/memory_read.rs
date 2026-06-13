//! `den`-side wiring for the memory read tools.
//!
//! Orchestration (arg parsing, validation, status diagnostic composition) lives
//! in `den-tools`; here we provide the concrete [`RoleMemoryStore`] — the native
//! SQLite path plus the legacy MemFS HTTP fallback — and thin wrappers that adapt
//! `DenToolInvocationContext` and map `DenError` back to `CustomError`.

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_tools::memory::{RoleMemoryEntryWrite, RoleMemoryStore};

use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        memory::{tools as sqlite_memory, MemoryStoreManager},
        memory_manager_head::MemfsWriteRoleMemoryEntryRequest,
        tools::{
            memfs::{
                fetch_role_memory_file, fetch_role_memory_status, fetch_role_memory_tree,
                memfs_http_client, search_role_memory, write_role_memory_entry,
            },
            prompt_memory::DenPromptMemoryStore,
            session::DenToolInvocationContext,
        },
    },
    errors::{CustomError, DenError},
};

/// Concrete [`RoleMemoryStore`] over the runtime config (native SQLite + legacy MemFS).
pub(crate) struct DenRoleMemoryStore<'a> {
    config: &'a Config,
}

impl<'a> DenRoleMemoryStore<'a> {
    pub(crate) fn new(config: &'a Config) -> Self {
        Self { config }
    }
}

#[async_trait]
impl RoleMemoryStore for DenRoleMemoryStore<'_> {
    async fn read(&self, bear_id: Uuid, role: BearProfile, path: &str) -> Result<Value, DenError> {
        if self.config.uses_native_agent_runtime() {
            let stores = MemoryStoreManager::new(self.config);
            let store = stores.store_for_bear(bear_id).await?;
            return sqlite_memory::sqlite_memory_read(&store, path)
                .await
                .map_err(CustomError::into_den);
        }
        let http = memfs_http_client("MemFS memory read client build failed")
            .map_err(CustomError::into_den)?;
        let response = fetch_role_memory_file(
            &http,
            &self.config.letta_memfs_service_url,
            bear_id,
            role.as_str(),
            path,
        )
        .await
        .map_err(CustomError::into_den)?;
        response
            .map(|value| {
                serde_json::to_value(value)
                    .map_err(|e| DenError::Parsing(format!("memory file JSON: {e}")))
            })
            .unwrap_or_else(|| {
                Ok(json!({
                    "ok": false,
                    "configured": false,
                    "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)"
                }))
            })
    }

    async fn browse(&self, bear_id: Uuid, role: BearProfile) -> Result<Value, DenError> {
        if self.config.uses_native_agent_runtime() {
            let stores = MemoryStoreManager::new(self.config);
            let store = stores.store_for_bear(bear_id).await?;
            return sqlite_memory::sqlite_memory_browse(&store, role.as_str())
                .await
                .map_err(CustomError::into_den);
        }
        let http = memfs_http_client("MemFS memory browse client build failed")
            .map_err(CustomError::into_den)?;
        let response = fetch_role_memory_tree(
            &http,
            &self.config.letta_memfs_service_url,
            bear_id,
            role.as_str(),
        )
        .await
        .map_err(CustomError::into_den)?;
        response
            .map(|value| {
                serde_json::to_value(value)
                    .map_err(|e| DenError::Parsing(format!("memory browse JSON: {e}")))
            })
            .unwrap_or_else(|| {
                Ok(json!({
                    "ok": false,
                    "configured": false,
                    "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)"
                }))
            })
    }

    async fn search(
        &self,
        bear_id: Uuid,
        role: BearProfile,
        query: &str,
        limit: i64,
    ) -> Result<Value, DenError> {
        if self.config.uses_native_agent_runtime() {
            let stores = MemoryStoreManager::new(self.config);
            let store = stores.store_for_bear(bear_id).await?;
            return sqlite_memory::sqlite_memory_search(&store, role.as_str(), query, limit)
                .await
                .map_err(CustomError::into_den);
        }
        let http = memfs_http_client("MemFS memory search client build failed")
            .map_err(CustomError::into_den)?;
        let response = search_role_memory(
            &http,
            &self.config.letta_memfs_service_url,
            bear_id,
            role.as_str(),
            query,
            Some(limit.clamp(1, 50) as usize),
        )
        .await
        .map_err(CustomError::into_den)?;
        response
            .map(|value| {
                serde_json::to_value(value)
                    .map_err(|e| DenError::Parsing(format!("memory search JSON: {e}")))
            })
            .unwrap_or_else(|| {
                Ok(json!({
                    "ok": false,
                    "configured": false,
                    "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)"
                }))
            })
    }

    async fn status_base(&self, bear_id: Uuid, role: BearProfile) -> Result<Value, DenError> {
        if self.config.uses_native_agent_runtime() {
            let stores = MemoryStoreManager::new(self.config);
            let store = stores.store_for_bear(bear_id).await?;
            return sqlite_memory::sqlite_memory_status(&store, role.as_str())
                .await
                .map_err(CustomError::into_den);
        }
        let http = memfs_http_client("MemFS memory status client build failed")
            .map_err(CustomError::into_den)?;
        let response = fetch_role_memory_status(
            &http,
            &self.config.letta_memfs_service_url,
            bear_id,
            role.as_str(),
        )
        .await
        .map_err(CustomError::into_den)?;
        let Some(response) = response else {
            return Ok(json!({
                "configured": false,
                "available": false,
                "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)",
            }));
        };
        Ok(json!({
            "configured": true,
            "available": response.ok,
            "bear_id": bear_id,
            "profile": role.as_str(),
            "canonical_tip": response.canonical_tip,
            "allowed_prefixes": response.allowed_prefixes,
            "file_count": response.file_count,
            "entry_count_by_kind": response.entry_count_by_kind,
            "registered_view_count": response.registered_view_count,
        }))
    }

    async fn write_entry(
        &self,
        bear_id: Uuid,
        role: BearProfile,
        entry: RoleMemoryEntryWrite,
    ) -> Result<Value, DenError> {
        if self.config.uses_native_agent_runtime() {
            let stores = MemoryStoreManager::new(self.config);
            return sqlite_memory::sqlite_write_profile_entry(
                &stores,
                self.config,
                bear_id,
                role.as_str(),
                &entry.kind,
                &entry.title,
                &entry.body,
                &entry.tags,
                entry.source,
                entry.author,
            )
            .await
            .map_err(CustomError::into_den);
        }
        let request = MemfsWriteRoleMemoryEntryRequest {
            kind: entry.kind,
            title: entry.title,
            body: entry.body,
            tags: entry.tags,
            refs: entry.refs,
            lifecycle: entry.lifecycle,
            source: entry.source,
            author: entry.author,
            conversation_id: entry.conversation_id,
            session_id: entry.session_id,
            acp_session_id: entry.acp_session_id,
            conversation_selection: entry.conversation_selection,
            runtime_target: entry.runtime_target,
            binding_id: entry.binding_id,
            profile: entry.profile,
            request_id: entry.request_id,
        };
        let http = memfs_http_client("MemFS memory entry client build failed")
            .map_err(CustomError::into_den)?;
        let response = write_role_memory_entry(
            &http,
            &self.config.letta_memfs_service_url,
            bear_id,
            role.as_str(),
            &request,
        )
        .await
        .map_err(CustomError::into_den)?;
        let Some(response) = response else {
            return Err(DenError::System(
                "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)".to_string(),
            ));
        };
        Ok(json!({
            "bear_id": bear_id,
            "profile": role.as_str(),
            "kind": response.kind,
            "entry_id": response.entry_id,
            "path": response.path,
            "commit": response.commit,
            "canonical_tip": response.canonical_tip,
            "view": response.view,
        }))
    }
}

pub(crate) async fn memory_status(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearProfile,
) -> Result<Value, CustomError> {
    let memory = DenRoleMemoryStore::new(config);
    let prompt = DenPromptMemoryStore::new(pool);
    den_tools::memory::memory_status(&memory, &prompt, context.bear_id, role)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn memory_status_value(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearProfile,
    pool: &PgPool,
) -> Result<Value, CustomError> {
    memory_status(pool, config, context, role).await
}

