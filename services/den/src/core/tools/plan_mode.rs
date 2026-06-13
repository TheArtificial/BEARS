//! `den`-side wiring for the ACP plan-mode tools.
//!
//! Argument parsing/validation and the static response envelopes now live in
//! `den_tools::plan_mode`; this module provides the concrete [`PlanModeOps`]
//! implementation (DB rows, mode switches, native/MemFS artifact writes,
//! `turn_state` rendering) and thin wrappers that the dispatcher calls. See
//! `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B).

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_tools::plan_mode::{
    PlanModeExitView, PlanModeOps, PlanModeStatusView, PlanModeView,
};

use crate::{
    config::Config,
    core::{
        acp_plan_mode::{
            self, AcpPlanModeRequestedBy, AcpPlanModeSessionRow, EnterPlanModeParams,
            SubmitPlanModeParams,
        },
        acp_sessions,
        acp_tools::{AcpResolvedSessionPolicy, AcpToolEnablementState},
        bears::BearProfile,
        memory::{tools as sqlite_memory, MemoryStoreManager},
        memory_manager_head::{write_memfs_role_memory_entry, MemfsWriteRoleMemoryEntryRequest},
        tools::{memfs::memfs_http_client, session::DenToolInvocationContext, support::clean_optional},
        turn_state,
    },
    errors::{CustomError, DenError},
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

/// Concrete [`PlanModeOps`] over the runtime pool/config/stores.
///
/// `config`/`stores` are only required by `exit` (artifact write); the dispatcher
/// supplies them for that path and leaves them `None` for the others.
pub(crate) struct DenPlanModeOps<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) config: Option<&'a Config>,
    pub(crate) stores: Option<&'a MemoryStoreManager>,
    pub(crate) workplan_payload: WorkplanPayloadFn,
    pub(crate) no_active_workplan: NoActiveWorkplanFn,
}

#[async_trait]
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
        .await
        .map_err(CustomError::into_den)?;
        acp_sessions::set_current_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            "plan",
        )
        .await
        .map_err(CustomError::into_den)?;
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
        .await
        .map_err(CustomError::into_den)?;
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
        .await
        .map_err(CustomError::into_den)?
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
        .await
        .map_err(CustomError::into_den)?;
        acp_sessions::set_current_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            "write",
        )
        .await
        .map_err(CustomError::into_den)?;
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
        let config = self.config.ok_or_else(|| {
            DenError::System("plan mode exit requires runtime config".to_string())
        })?;
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
        .await
        .map_err(CustomError::into_den)?
        .ok_or_else(|| {
            DenError::NotFound("active ACP plan mode session not found".to_string())
        })?;
        let artifact_path = if config.uses_native_agent_runtime() {
            let artifact_id = format!("plan-mode-{}", current_plan.id);
            let logical_path = format!("pair/plans/{artifact_id}.md");
            let written = sqlite_memory::sqlite_write_at_path(
                stores,
                config,
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
            .await
            .map_err(CustomError::into_den)?;
            written
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or(&logical_path)
                .to_string()
        } else {
            let memory_request = MemfsWriteRoleMemoryEntryRequest {
                kind: "plan".to_string(),
                title: title.to_string(),
                body: markdown,
                tags: vec!["plan-mode".to_string(), "implementation-plan".to_string()],
                refs: None,
                lifecycle: Some(json!({ "scope": "role-local", "retention": "durable" })),
                source: Some(json!({
                    "tool": crate::core::tools::constants::DEN_PLAN_MODE_EXIT,
                    "acp_session_id": acp_session_id,
                    "conversation_id": clean_optional(&context.conversation_id),
                })),
                author: context.username.clone(),
                conversation_id: clean_optional(&context.conversation_id),
                session_id: Some(acp_session_id.to_string()),
                acp_session_id: Some(acp_session_id.to_string()),
                conversation_selection: context.conversation_selection.clone(),
                runtime_target: context.runtime_target.clone(),
                binding_id: Some(context.binding_id.clone()),
                profile: Some(BearProfile::Pair.as_str().to_string()),
                request_id: context.request_id.clone(),
            };
            let http = memfs_http_client("MemFS plan artifact client build failed")
                .map_err(CustomError::into_den)?;
            let memfs_response = write_memfs_role_memory_entry(
                &http,
                &config.letta_memfs_service_url,
                context.bear_id,
                BearProfile::Pair.as_str(),
                &memory_request,
            )
            .await
            .map_err(CustomError::into_den)?;
            let Some(memfs_response) = memfs_response else {
                return Err(DenError::System(
                    "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)".to_string(),
                ));
            };
            memfs_response.path
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
        .await
        .map_err(CustomError::into_den)?;
        acp_sessions::set_current_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            "plan",
        )
        .await
        .map_err(CustomError::into_den)?;
        let storage = if config.uses_native_agent_runtime() {
            "sqlite"
        } else {
            "memfs"
        };
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
        .await
        .map_err(CustomError::into_den)?;
        acp_sessions::set_current_mode(
            self.pool,
            context.user_id,
            context.bear_id,
            acp_session_id,
            "ask",
        )
        .await
        .map_err(CustomError::into_den)?;
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

fn no_active_placeholder() -> Value {
    Value::Null
}

pub(crate) async fn enter_plan_mode(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
    plan_mode_workplan_payload: WorkplanPayloadFn,
) -> Result<Value, CustomError> {
    let runtime = DenPlanModeOps {
        pool,
        config: None,
        stores: None,
        workplan_payload: plan_mode_workplan_payload,
        no_active_workplan: no_active_placeholder,
    };
    den_tools::plan_mode::enter_plan_mode(&runtime, context, arguments)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn plan_mode_status(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    plan_mode_workplan_payload: WorkplanPayloadFn,
    no_active_workplan_payload: NoActiveWorkplanFn,
) -> Result<Value, CustomError> {
    let runtime = DenPlanModeOps {
        pool,
        config: None,
        stores: None,
        workplan_payload: plan_mode_workplan_payload,
        no_active_workplan: no_active_workplan_payload,
    };
    den_tools::plan_mode::plan_mode_status(&runtime, context)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn record_plan_approval(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
    plan_mode_workplan_payload: WorkplanPayloadFn,
) -> Result<Value, CustomError> {
    let runtime = DenPlanModeOps {
        pool,
        config: None,
        stores: None,
        workplan_payload: plan_mode_workplan_payload,
        no_active_workplan: no_active_placeholder,
    };
    den_tools::plan_mode::record_plan_approval(&runtime, context, arguments)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn exit_plan_mode(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    arguments: Value,
    plan_mode_workplan_payload: WorkplanPayloadFn,
) -> Result<Value, CustomError> {
    let runtime = DenPlanModeOps {
        pool,
        config: Some(config),
        stores: Some(stores),
        workplan_payload: plan_mode_workplan_payload,
        no_active_workplan: no_active_placeholder,
    };
    den_tools::plan_mode::exit_plan_mode(&runtime, context, arguments)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn cancel_plan_mode(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    arguments: Value,
    plan_mode_workplan_payload: WorkplanPayloadFn,
) -> Result<Value, CustomError> {
    let runtime = DenPlanModeOps {
        pool,
        config: None,
        stores: None,
        workplan_payload: plan_mode_workplan_payload,
        no_active_workplan: no_active_placeholder,
    };
    den_tools::plan_mode::cancel_plan_mode(&runtime, context, arguments)
        .await
        .map_err(CustomError::from)
}
