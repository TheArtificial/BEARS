//! Capability seam for the `session_info` / `bear_environment` orientation tools.
//!
//! The payload shaping is pure (see `payloads.rs`); [`EnvironmentOps`] exposes the
//! few runtime-coupled inputs those builders need — the memory-status snapshot,
//! the optional ACP adapter-runtime fetch, and two config flags. The `den`
//! implementation owns the DB/HTTP access; membership/user lookups come from
//! [`crate::tools::identity::BearDirectory`].

use serde_json::Value;

use crate::{BearProfile, DenError};

use crate::tools::context::DenToolInvocationContext;

// Native async fn in trait: workspace-internal, consumed via generic bounds /
// concrete impls only (never `dyn`), so Send flows through monomorphization.
#[allow(async_fn_in_trait)]
pub trait EnvironmentOps: Send + Sync {
    /// Whether the native (SQLite) agent runtime is active.
    fn uses_native_runtime(&self) -> bool;

    /// Whether the legacy MemFS sidecar is configured.
    fn memfs_configured(&self) -> bool;

    /// Raw memory-status snapshot for this Bear/role (may error; callers degrade).
    async fn memory_status_value(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
    ) -> Result<Value, DenError>;

    /// Optional ACP adapter-runtime enrichment for the current session.
    async fn fetch_acp_adapter_environment(
        &self,
        context: &DenToolInvocationContext,
    ) -> Result<Option<Value>, DenError>;
}
