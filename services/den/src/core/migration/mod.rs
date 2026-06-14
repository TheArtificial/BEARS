//! One-time backfill helpers for Letta/MemFS → Den-native stores (Phase 8).

use sqlx::PgPool;

use crate::{
    config::Config,
    errors::CustomError,
};
use den_runtime::memory::MemoryStoreManager;

pub struct NativeRuntimeBackfill;

impl NativeRuntimeBackfill {
    pub async fn backfill_memfs_placeholder(
        _pool: &PgPool,
        _config: &Config,
        _stores: &MemoryStoreManager,
    ) -> Result<serde_json::Value, CustomError> {
        Ok(serde_json::json!({
            "status": "not_run",
            "message": "MemFS backfill is deferred until Phase 8 operator runbook"
        }))
    }

    pub async fn backfill_letta_conversations_placeholder(
        _pool: &PgPool,
    ) -> Result<serde_json::Value, CustomError> {
        Ok(serde_json::json!({
            "status": "not_run",
            "message": "Letta conversation backfill is deferred until Phase 8 operator runbook"
        }))
    }
}
