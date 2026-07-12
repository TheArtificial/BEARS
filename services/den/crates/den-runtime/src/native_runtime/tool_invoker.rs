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
use std::sync::{Arc, OnceLock};

use den_core::tools::context::DenToolInvocationContext;
use den_core::{config::Config, DenError};

use den_memory::MemoryStoreManager;

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

static TOOL_INVOKER: OnceLock<Arc<dyn RuntimeToolInvoker>> = OnceLock::new();

/// Install the process-wide builtin-Den-tool invoker.
///
/// The HTTP edges (`den-api`/`adapter edge`/`den-web`) execute builtin Den tools (the
/// `/internal/den-tools/invoke` endpoint, armature runtime-local tool calls, the web
/// chat turn) but depend only on the [`RuntimeToolInvoker`] trait — not on the
/// concrete den-side tool composition (`DenToolContext` + executors), which lives
/// in the `den` binary. The binary injects its concrete invoker here at startup,
/// so the edges stay free of that dependency. Idempotent; the first install wins.
pub fn set_tool_invoker(invoker: Arc<dyn RuntimeToolInvoker>) {
    let _ = TOOL_INVOKER.set(invoker);
}

/// The installed [`set_tool_invoker`] invoker, if any. `None` before the binary
/// installs one (e.g. in unit tests that never execute a builtin Den tool).
pub fn tool_invoker() -> Option<Arc<dyn RuntimeToolInvoker>> {
    TOOL_INVOKER.get().cloned()
}
