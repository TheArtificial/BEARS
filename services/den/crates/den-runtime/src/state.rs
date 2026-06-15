//! Shared application state for the Den HTTP edges.
//!
//! `DenState` lives in `den-runtime` — below every HTTP edge — so that neither
//! the JSON/REST edge (`den-api`) nor the ACP edge (`den-acp`) owns the state
//! both surfaces share. Per ADR-0043 the runtime core is protocol-agnostic: this
//! state holds runtime handles (the database pool, configuration, the model
//! client, per-Bear memory stores, and the process-local turn coordinators), not
//! any wire-protocol concept. Each edge depends on `den-runtime` and builds the
//! state via [`DenState::new`].

use std::sync::Arc;

use sqlx::PgPool;

use den_core::config::Config;

use crate::{
    bifrost::BifrostClient, memory::MemoryStoreManager, tool_turns::ToolTurnCoordinator,
    turn_controller::ActiveTurnCancelRegistry,
};

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
            tool_turns: ToolTurnCoordinator::new(),
            acp_turn_cancellations: ActiveTurnCancelRegistry::new(),
            memory_stores,
        }
    }
}
