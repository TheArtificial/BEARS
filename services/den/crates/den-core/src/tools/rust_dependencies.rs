//! Hosted Rust dependency-preparation tool seam.
//!
//! This is deliberately a Den-hosted capability: the restricted sandbox never
//! receives outbound network authority or arbitrary Cargo command execution.

use serde_json::{json, Value};

use crate::tools::{
    arguments::PrepareRustDependenciesArguments, context::DenToolInvocationContext,
};
use crate::DenError;

#[allow(async_fn_in_trait)]
pub trait RustDependencyPreparationOps: Send + Sync {
    async fn prepare_rust_dependencies(
        &self,
        context: &DenToolInvocationContext,
        arguments: PrepareRustDependenciesArguments,
    ) -> Result<Value, DenError>;
}

pub async fn prepare_rust_dependencies(
    ops: &impl RustDependencyPreparationOps,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, DenError> {
    let arguments = serde_json::from_value(arguments)?;
    ops.prepare_rust_dependencies(context, arguments).await
}

/// Used by runtimes that do not own an active work-run executor. Keeping this
/// explicit prevents the descriptor from implying sandbox network access.
pub fn unavailable_result() -> Value {
    json!({
        "ok": false,
        "status": "unavailable",
        "content": "Rust dependency preparation is available only during an active work run.",
    })
}
