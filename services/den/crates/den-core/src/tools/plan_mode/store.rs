//! Capability seam for ACP plan-mode operations.
//!
//! [`PlanModeOps`] abstracts the runtime side of plan mode: persisting plan-mode
//! session state, switching the ACP session permission mode, and writing the plan
//! artifact. The `den-tools` executors own argument parsing/validation and the
//! static envelope text; the `den` implementation owns the `acp_plan_mode` DB
//! rows, `turn_state` rendering, and native-vs-MemFS artifact writes. See
//! `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B).

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use den_core::DenError;

use crate::context::DenToolInvocationContext;

/// Common rendered view returned by enter/approve/cancel transitions.
///
/// Each field is already serialized so `den-tools` only assembles the outer
/// envelope (domain marker, mode update, static instructions).
pub struct PlanModeView {
    pub workplan: Value,
    pub plan_mode: Value,
    pub workflow_state: Value,
}

/// Rendered view for the read-only plan-mode status surface.
pub struct PlanModeStatusView {
    pub workplan: Value,
    pub plan_mode: Value,
    pub active: bool,
}

/// Rendered view for a submitted plan artifact (exit transition).
pub struct PlanModeExitView {
    pub workplan: Value,
    pub plan_mode: Value,
    pub workflow_state: Value,
    pub artifact_path: String,
    pub storage: String,
    pub submitted_plan: Value,
}

#[async_trait]
pub trait PlanModeOps: Send + Sync {
    /// Enter plan mode for the session and switch it to the read-only `plan` mode.
    async fn enter(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
        reason: String,
        previous_permission_mode: Option<String>,
    ) -> Result<PlanModeView, DenError>;

    /// Report the active plan-mode session (if any) for this ACP session.
    async fn status(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
    ) -> Result<PlanModeStatusView, DenError>;

    /// Record human approval of a submitted plan and switch to `write` mode.
    async fn record_approval(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
        plan_mode_id: Option<Uuid>,
    ) -> Result<PlanModeView, DenError>;

    /// Persist the submitted plan artifact and return to `plan` mode.
    async fn exit(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
        plan_mode_id: Option<Uuid>,
        title: &str,
        body: &str,
    ) -> Result<PlanModeExitView, DenError>;

    /// Cancel the active plan-mode session and switch to `ask` mode.
    async fn cancel(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
        plan_mode_id: Option<Uuid>,
    ) -> Result<PlanModeView, DenError>;
}
