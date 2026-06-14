//! Tool-dispatch abstraction for the native runtime.
//!
//! The native runtime executes builtin Den tools mid-turn, but the concrete tool
//! wiring (the `DenToolContext` aggregate and its executors over `bears`, `memory`,
//! `reflection`, `work_plans`, `docket`, …) lives in the `den` binary. To keep
//! `den-runtime` free of that dependency, the runtime depends only on this trait and
//! the `den` binary injects a concrete invoker at the turn boundary.

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;

use den_core::{config::Config, DenError};
use den_core::tools::context::DenToolInvocationContext;

use crate::memory::MemoryStoreManager;

/// Dispatches a single builtin Den tool call to its concrete executor.
#[async_trait]
pub trait RuntimeToolInvoker: Send + Sync {
    async fn invoke(
        &self,
        pool: &PgPool,
        config: &Config,
        stores: &MemoryStoreManager,
        tool_name: &str,
        arguments: Value,
        context: DenToolInvocationContext,
    ) -> Result<Value, DenError>;
}
