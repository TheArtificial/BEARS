//! Concrete [`RuntimeToolInvoker`] bridging the native runtime's tool-dispatch
//! abstraction to the in-`den` `DenToolContext` aggregate.
//!
//! `den-runtime` depends only on the [`RuntimeToolInvoker`] trait; the `den` binary
//! injects this implementation at the web-chat turn boundary so the runtime can run
//! builtin Den tools without `den-runtime` depending on the concrete tool wiring.

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;

use den_core::tools::context::DenToolInvocationContext;
use den_core::{config::Config, DenError};
use den_runtime::memory::MemoryStoreManager;
use den_runtime::native_runtime::RuntimeToolInvoker;

use crate::core::tools::session::invoke_den_tool;
use crate::errors::CustomError;

pub struct DenRuntimeToolInvoker;

#[async_trait]
impl RuntimeToolInvoker for DenRuntimeToolInvoker {
    async fn invoke(
        &self,
        pool: &PgPool,
        config: &Config,
        stores: &MemoryStoreManager,
        tool_name: &str,
        arguments: Value,
        context: DenToolInvocationContext,
    ) -> Result<Value, DenError> {
        invoke_den_tool(pool, config, stores, tool_name, arguments, context)
            .await
            .map_err(CustomError::into_den)
    }
}
