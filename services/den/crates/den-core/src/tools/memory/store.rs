//! The `RoleMemoryStore` capability seam: canonical per-Bear, per-role memory.
//!
//! Hides the SQLite `BearMemoryStore` (and the legacy MemFS HTTP fallback) behind
//! a trait returning already-shaped tool JSON. The `den` crate implements it over
//! `MemoryStoreManager` + `memory::tools::sqlite_*`. Methods are grown as executor
//! groups migrate (read surface first; write surface follows).
//! See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

use async_trait::async_trait;
use den_core::{BearProfile, DenError};
use serde_json::Value;
use uuid::Uuid;

use super::RoleMemoryEntryWrite;

#[async_trait]
pub trait RoleMemoryStore: Send + Sync {
    /// Read records at a logical path (tool-shaped JSON).
    async fn read(&self, bear_id: Uuid, role: BearProfile, path: &str) -> Result<Value, DenError>;

    /// Browse the role memory tree (tool-shaped JSON).
    async fn browse(&self, bear_id: Uuid, role: BearProfile) -> Result<Value, DenError>;

    /// Search role memory (tool-shaped JSON). `limit` is already clamped.
    async fn search(
        &self,
        bear_id: Uuid,
        role: BearProfile,
        query: &str,
        limit: i64,
    ) -> Result<Value, DenError>;

    /// Base memory-status JSON **without** the prompt-memory diagnostic; the
    /// `memory_status` executor composes that diagnostic on top.
    async fn status_base(&self, bear_id: Uuid, role: BearProfile) -> Result<Value, DenError>;

    /// Persist a role-memory entry; returns tool-shaped JSON.
    async fn write_entry(
        &self,
        bear_id: Uuid,
        role: BearProfile,
        entry: RoleMemoryEntryWrite,
    ) -> Result<Value, DenError>;
}
