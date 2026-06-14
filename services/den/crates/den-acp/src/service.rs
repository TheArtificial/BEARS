//! Shared application state for the API + ACP HTTP surface.
//!
//! `ApiState` lives in `den-acp` (the lower of the two HTTP edges) per the v1.5+
//! split decision (Option B): the ACP surface and the `den-api` v1 surface both
//! need it, and `den-api` *mounts* the ACP router, so `den-api` depends on
//! `den-acp` and uses `den_acp::service::ApiState`. The ACP-specific coordinator
//! fields stay crate-private; `den-api`'s `create_api_app` builds the state via
//! [`ApiState::new`].

use std::sync::Arc;

use sqlx::PgPool;

use den_core::config::Config;
use den_runtime::{bifrost::BifrostClient, memory::MemoryStoreManager};

/// Application state for the API service.
///
/// Contains shared resources needed by API + ACP endpoints including database
/// connections, configuration, the Bifrost model client, per-Bear memory stores,
/// and the process-local ACP turn coordinators.
#[derive(Clone)]
pub struct ApiState {
    /// Database connection pool for API operations
    pub sqlx_pool: PgPool,
    /// Shared immutable runtime configuration.
    pub config: Arc<Config>,
    /// Shared Bifrost model metadata client.
    pub bifrost: Arc<BifrostClient>,
    /// Process-local active ACP direct tool turns.
    pub(crate) acp_tool_turns: den_runtime::acp_tool_turns::AcpToolTurnCoordinator,
    /// Process-local active ACP stream cancellation signals.
    pub(crate) acp_turn_cancellations:
        den_runtime::acp_turn_controller::AcpActiveTurnCancelRegistry,
    /// Per-Bear SQLite memory stores (native runtime cognition).
    pub memory_stores: MemoryStoreManager,
}

impl ApiState {
    /// Build the API/ACP state, initializing the process-local ACP turn
    /// coordinators. Called by `den-api`'s `create_api_app` composition root.
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
            acp_tool_turns: den_runtime::acp_tool_turns::AcpToolTurnCoordinator::new(),
            acp_turn_cancellations:
                den_runtime::acp_turn_controller::AcpActiveTurnCancelRegistry::new(),
            memory_stores,
        }
    }
}
