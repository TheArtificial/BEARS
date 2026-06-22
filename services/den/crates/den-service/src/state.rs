//! Shared application state for the Den HTTP edges.
//!
//! `DenState` lives in `den-service` — below every HTTP edge and below
//! `den-runtime` — so the native runtime implementation is not the owner of
//! shared application wiring. The state holds service handles (database pool,
//! configuration, model metadata client, per-Bear memory stores, and process-local
//! turn coordinators), not runtime execution logic.

use std::sync::Arc;

use sqlx::PgPool;

use den_core::config::Config;

use crate::{
    bifrost::{new_catalog_store, BifrostCatalogStore, BifrostClient},
    tool_turns::ToolTurnCoordinator,
    turn_controller::ActiveTurnCancelRegistry,
};
use den_memory::MemoryStoreManager;

/// Shared state for the Den HTTP surfaces.
///
/// Contains resources needed by every edge: the database connection pool,
/// immutable runtime configuration, the Bifrost model-metadata client, per-Bear
/// SQLite memory stores, and the process-local turn coordinators.
#[derive(Clone)]
pub struct DenState {
    /// Database connection pool.
    pub sqlx_pool: PgPool,
    /// Shared immutable runtime configuration.
    pub config: Arc<Config>,
    /// Shared Bifrost model metadata client.
    pub bifrost: Arc<BifrostClient>,
    /// Shared runtime model catalog snapshot.
    pub bifrost_catalog: BifrostCatalogStore,
    /// Process-local active direct tool turns.
    pub tool_turns: ToolTurnCoordinator,
    /// Process-local active stream cancellation signals.
    pub acp_turn_cancellations: ActiveTurnCancelRegistry,
    /// Per-Bear SQLite memory stores (native runtime cognition).
    pub memory_stores: MemoryStoreManager,
}

impl DenState {
    /// Build the shared state, initializing the process-local turn coordinators.
    /// Called by each edge's composition root (e.g. `den-api`'s `create_api_app`).
    #[must_use]
    pub fn new(
        sqlx_pool: PgPool,
        config: Arc<Config>,
        bifrost: Arc<BifrostClient>,
        memory_stores: MemoryStoreManager,
    ) -> Self {
        Self {
            sqlx_pool,
            config,
            bifrost,
            bifrost_catalog: new_catalog_store(),
            tool_turns: ToolTurnCoordinator::new(),
            acp_turn_cancellations: ActiveTurnCancelRegistry::new(),
            memory_stores,
        }
    }
}
