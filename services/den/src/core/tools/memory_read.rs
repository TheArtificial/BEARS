//! `den`-side wiring for the memory read tools.
//!
//! Orchestration (arg parsing, validation, status diagnostic composition) lives
//! in `den-tools`; here we provide the concrete [`RoleMemoryStore`] over the
//! native per-Bear SQLite store, plus thin wrappers that adapt
//! `DenToolInvocationContext` and map `DenError` back to `CustomError`.

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use den_tools::memory::{RoleMemoryEntryWrite, RoleMemoryStore};

use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        memory::{tools as sqlite_memory, MemoryStoreManager},
        tools::{prompt_memory::DenPromptMemoryStore, session::DenToolInvocationContext},
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
    async fn read(&self, bear_id: Uuid, _role: BearProfile, path: &str) -> Result<Value, DenError> {
        let stores = MemoryStoreManager::new(self.config);
        let store = stores.store_for_bear(bear_id).await?;
        sqlite_memory::sqlite_memory_read(&store, path)
            .await
            .map_err(CustomError::into_den)
    }

    async fn browse(&self, bear_id: Uuid, role: BearProfile) -> Result<Value, DenError> {
        let stores = MemoryStoreManager::new(self.config);
        let store = stores.store_for_bear(bear_id).await?;
        sqlite_memory::sqlite_memory_browse(&store, role.as_str())
            .await
            .map_err(CustomError::into_den)
    }

    async fn search(
        &self,
        bear_id: Uuid,
        role: BearProfile,
        query: &str,
        limit: i64,
    ) -> Result<Value, DenError> {
        let stores = MemoryStoreManager::new(self.config);
        let store = stores.store_for_bear(bear_id).await?;
        sqlite_memory::sqlite_memory_search(&store, role.as_str(), query, limit)
            .await
            .map_err(CustomError::into_den)
    }

    async fn status_base(&self, bear_id: Uuid, role: BearProfile) -> Result<Value, DenError> {
        let stores = MemoryStoreManager::new(self.config);
        let store = stores.store_for_bear(bear_id).await?;
        sqlite_memory::sqlite_memory_status(&store, role.as_str())
            .await
            .map_err(CustomError::into_den)
    }

    async fn write_entry(
        &self,
        bear_id: Uuid,
        role: BearProfile,
        entry: RoleMemoryEntryWrite,
    ) -> Result<Value, DenError> {
        let stores = MemoryStoreManager::new(self.config);
        sqlite_memory::sqlite_write_profile_entry(
            &stores,
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
        .map_err(CustomError::into_den)
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

