//! `den`-side wiring for the ACP plan-mode tools.
//!
//! Argument parsing/validation and the static response envelopes now live in
//! `den_core::tools::plan_mode`; this module provides the concrete [`PlanModeOps`]
//! implementation (DB rows, mode switches, native SQLite artifact writes,
//! `turn_state` rendering), wired into the dispatcher via `DenToolContext`. See
//! `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B).

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::tools::plan_mode::{
    PlanModeExitView, PlanModeOps, PlanModeStatusView, PlanModeView,
};

use crate::{
    errors::DenError,
    core::{
        tools::{session::DenToolInvocationContext, support::clean_optional},
    },
};
use den_runtime::{
    acp_plan_mode::{
            self, AcpPlanModeRequestedBy, AcpPlanModeSessionRow, EnterPlanModeParams,
            SubmitPlanModeParams,
        },
    acp_sessions,
    acp_tools::{AcpResolvedSessionPolicy, AcpToolEnablementState},
    bears::BearProfile,
    memory::{tools as sqlite_memory, MemoryStoreManager},
    turn_state,
};

type WorkplanPayloadFn = fn(&AcpPlanModeSessionRow) -> Value;
type NoActiveWorkplanFn = fn() -> Value;

fn workflow_state_json(
    mode_label: &'static str,
    tool_enablement: AcpToolEnablementState,
    plan_mode_state: String,
) -> Value {
    turn_state::turn_state_json(
        &AcpResolvedSessionPolicy {
            mode_label,
            tool_enablement,
            plan_mode_state: Some(plan_mode_state),
        },
        None,
    )
}

/// Concrete [`PlanModeOps`] over the runtime pool/stores.
///
/// `stores` is only required by `exit` (artifact write); the dispatcher
/// supplies it for that path and leaves it `None` for the others.
pub(crate) struct DenPlanModeOps<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) stores: Option<&'a MemoryStoreManager>,
    pub(crate) workplan_payload: WorkplanPayloadFn,
    pub(crate) no_active_workplan: NoActiveWorkplanFn,
}

impl PlanModeOps for DenPlanModeOps<'_> {
    async fn enter(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
        reason: String,
        previous_permission_mode: Option<String>,
    ) -> Result<PlanModeView, DenError> {
        let row = acp_plan_mode::enter_plan_mode(
            self.pool,
            EnterPlanModeParams {
                user_id: context.user_id,
                bear_id: context.bear_id,
                bear_slug: context.bear_slug.clone(),
                acp_session_id: acp_session_id.to_string(),
                reason,
                requested_by: AcpPlanModeRequestedBy::Pair,
                previous_permission_mode,
            },
        )
        .await?;
        acp_sessions::set_current_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            "plan",
        )
        .await?;
        Ok(PlanModeView {
            workplan: (self.workplan_payload)(&row),
            workflow_state: workflow_state_json(
                "Plan",
                AcpToolEnablementState::ReadOnly,
                row.state.clone(),
            ),
            plan_mode: serde_json::to_value(&row)?,
        })
    }

    async fn status(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
    ) -> Result<PlanModeStatusView, DenError> {
        let row = acp_plan_mode::active_for_session(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
        )
        .await?;
        let workplan = row
            .as_ref()
            .map(self.workplan_payload)
            .unwrap_or_else(self.no_active_workplan);
        Ok(PlanModeStatusView {
            workplan,
            active: row.is_some(),
            plan_mode: serde_json::to_value(&row)?,
        })
    }

    async fn record_approval(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
        plan_mode_id: Option<Uuid>,
    ) -> Result<PlanModeView, DenError> {
        let current = acp_plan_mode::get_for_session(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            plan_mode_id,
        )
        .await?
        .ok_or_else(|| {
            DenError::NotFound("submitted ACP plan mode session not found".to_string())
        })?;
        if current.state != "submitted" {
            return Err(DenError::ValidationError(format!(
                "plan approval requires a submitted plan; current state is {}",
                current.state
            )));
        }
        let row = acp_plan_mode::approve_plan_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            current.id,
        )
        .await?;
        acp_sessions::set_current_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            "write",
        )
        .await?;
        Ok(PlanModeView {
            workplan: (self.workplan_payload)(&row),
            workflow_state: workflow_state_json(
                "Write",
                AcpToolEnablementState::AllTools,
                row.state.clone(),
            ),
            plan_mode: serde_json::to_value(&row)?,
        })
    }

    async fn exit(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
        plan_mode_id: Option<Uuid>,
        title: &str,
        body: &str,
    ) -> Result<PlanModeExitView, DenError> {
        let stores = self.stores.ok_or_else(|| {
            DenError::System("plan mode exit requires memory stores".to_string())
        })?;
        let markdown = acp_plan_mode::render_plan_artifact_markdown(title, body);
        let current_plan = acp_plan_mode::get_for_session(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            plan_mode_id,
        )
        .await?
        .ok_or_else(|| {
            DenError::NotFound("active ACP plan mode session not found".to_string())
        })?;
        let artifact_path = {
            let artifact_id = format!("plan-mode-{}", current_plan.id);
            let logical_path = format!("pair/plans/{artifact_id}.md");
            let written = sqlite_memory::sqlite_write_at_path(
                stores,
                context.bear_id,
                &logical_path,
                BearProfile::Pair.as_str(),
                title,
                &markdown,
                json!({
                    "kind": "plan",
                    "tags": ["plan-mode", "implementation-plan"],
                    "content_class": "workplan_artifact",
                    "source": {
                        "tool": crate::core::tools::constants::DEN_PLAN_MODE_EXIT,
                        "acp_session_id": acp_session_id,
                        "conversation_id": clean_optional(&context.conversation_id),
                    },
                }),
            )
            .await?;
            written
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or(&logical_path)
                .to_string()
        };
        let row = acp_plan_mode::submit_plan_artifact(
            self.pool,
            SubmitPlanModeParams {
                user_id: context.user_id,
                bear_id: context.bear_id,
                acp_session_id: acp_session_id.to_string(),
                plan_mode_id: Some(current_plan.id),
                title: title.to_string(),
                body: body.to_string(),
                artifact_path: artifact_path.clone(),
                approval_request_id: Some(format!("plan-mode-{}", current_plan.id)),
            },
        )
        .await?;
        acp_sessions::set_current_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            "plan",
        )
        .await?;
        let storage = "sqlite";
        Ok(PlanModeExitView {
            workplan: (self.workplan_payload)(&row),
            workflow_state: workflow_state_json(
                "Plan",
                AcpToolEnablementState::ReadOnly,
                row.state.clone(),
            ),
            submitted_plan: json!({
                "title": row.plan_title,
                "body": row.plan_body,
                "artifact_path": row.plan_artifact_path,
            }),
            artifact_path,
            storage: storage.to_string(),
            plan_mode: serde_json::to_value(&row)?,
        })
    }

    async fn cancel(
        &self,
        context: &DenToolInvocationContext,
        acp_session_id: &str,
        plan_mode_id: Option<Uuid>,
    ) -> Result<PlanModeView, DenError> {
        let row = acp_plan_mode::cancel_plan_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            plan_mode_id,
        )
        .await?;
        acp_sessions::set_current_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            "ask",
        )
        .await?;
        Ok(PlanModeView {
            workplan: (self.workplan_payload)(&row),
            workflow_state: workflow_state_json(
                "Ask",
                AcpToolEnablementState::ReadOnly,
                row.state.clone(),
            ),
            plan_mode: serde_json::to_value(&row)?,
        })
    }
}

