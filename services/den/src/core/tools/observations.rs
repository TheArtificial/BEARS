//! `den`-side wiring for `observation_write`.
//!
//! Orchestration lives in `den-tools`; the concrete [`MemoryReviewStore`] is
//! shared with the memory-review tools (see [`crate::core::tools::memory_review`]).

use serde_json::Value;
use sqlx::PgPool;

use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        memory::MemoryStoreManager,
        tools::{memory_review::DenMemoryReviewStore, session::DenToolInvocationContext},
    },
    errors::CustomError,
};

pub(crate) async fn write_observation(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let store = DenMemoryReviewStore::new(pool, config, stores);
    den_tools::review::write_observation(&store, context, role, arguments)
        .await
        .map_err(CustomError::from)
}
