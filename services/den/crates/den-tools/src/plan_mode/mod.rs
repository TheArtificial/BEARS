//! ACP plan-mode tool executors.
//!
//! These executors own argument parsing/validation, the ACP-session-id
//! requirement, and the static response envelope (domain marker, mode update,
//! human-facing instruction lists). All runtime side effects (DB rows, mode
//! switches, plan-artifact writes, `turn_state` rendering) flow through the
//! [`PlanModeOps`] seam, whose `den` implementation owns the den-only types.

mod store;

pub use store::{PlanModeExitView, PlanModeOps, PlanModeStatusView, PlanModeView};

use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use den_core::DenError;

use crate::{context::DenToolInvocationContext, memory::source_acp_session_id, validation};

#[derive(Debug, Deserialize)]
pub struct PlanModeEnterArguments {
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub previous_permission_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PlanModeRecordApprovalArguments {
    pub approval_text: String,
    #[serde(default)]
    pub plan_mode_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct PlanModeExitArguments {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub plan_mode_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct PlanModeCancelArguments {
    #[serde(default)]
    pub plan_mode_id: Option<Uuid>,
}

fn require_acp_session(
    context: &DenToolInvocationContext,
    purpose: &str,
) -> Result<String, DenError> {
    source_acp_session_id(context)
        .ok_or_else(|| DenError::ValidationError(format!("ACP session id is required for {purpose}")))
}

pub async fn enter_plan_mode(
    ops: &impl PlanModeOps,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, DenError> {
    let args: PlanModeEnterArguments = serde_json::from_value(arguments)?;
    let acp_session_id = require_acp_session(context, "plan mode")?;
    let view = ops
        .enter(
            context,
            &acp_session_id,
            args.reason,
            args.previous_permission_mode,
        )
        .await?;
    Ok(json!({
        "domain": "workplan",
        "workplan": view.workplan,
        "plan_mode": view.plan_mode,
        "workflow_state": view.workflow_state,
        "mode_update": "plan",
        "instructions": [
            "Plan mode is active for this ACP session.",
            "Inspect, read, search, and use read-only Den tools as needed.",
            "Do not mutate workspace files, run non-read-only shell commands, or perform external side effects until the submitted plan is approved.",
            "Call den.plan_mode.exit with a concise markdown implementation plan when ready for user approval."
        ]
    }))
}

pub async fn plan_mode_status(
    ops: &impl PlanModeOps,
    context: &DenToolInvocationContext,
) -> Result<Value, DenError> {
    let acp_session_id = require_acp_session(context, "plan mode")?;
    let view = ops.status(context, &acp_session_id).await?;
    Ok(json!({
        "domain": "workplan",
        "bear_id": context.bear_id,
        "acp_session_id": acp_session_id,
        "workplan": view.workplan,
        "plan_mode": view.plan_mode,
        "active": view.active,
    }))
}

pub async fn record_plan_approval(
    ops: &impl PlanModeOps,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, DenError> {
    let args: PlanModeRecordApprovalArguments = serde_json::from_value(arguments)?;
    let approval_text = validation::validate_bounded_text("approval_text", &args.approval_text, 1, 1000)?;
    let acp_session_id = require_acp_session(context, "plan approval")?;
    let view = ops
        .record_approval(context, &acp_session_id, args.plan_mode_id)
        .await?;
    Ok(json!({
        "domain": "workplan",
        "ok": true,
        "workplan": view.workplan,
        "plan_mode": view.plan_mode,
        "workflow_state": view.workflow_state,
        "mode_update": "write",
        "approval_text": approval_text,
        "content": "Plan approved by the authenticated human. Write mode is now enabled; implementation may proceed subject to normal ACP tool approvals.",
    }))
}

pub async fn exit_plan_mode(
    ops: &impl PlanModeOps,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, DenError> {
    let args: PlanModeExitArguments = serde_json::from_value(arguments)?;
    let acp_session_id = require_acp_session(context, "plan mode")?;
    let title = validation::validate_bounded_text("title", &args.title, 1, 200)?;
    let body = validation::validate_bounded_text("body", &args.body, 1, 50_000)?;
    let view = ops
        .exit(context, &acp_session_id, args.plan_mode_id, &title, &body)
        .await?;
    Ok(json!({
        "domain": "workplan",
        "workplan": view.workplan,
        "plan_mode": view.plan_mode,
        "workflow_state": view.workflow_state,
        "artifact": {
            "domain": "workplan",
            "content_class": "workplan_artifact",
            "path": view.artifact_path,
            "storage": view.storage,
        },
        "approval_required": false,
        "mode_update": "plan",
        "submitted_plan": view.submitted_plan,
        "instructions": [
            "Present this plan artifact to the user if useful.",
            "If the authenticated human clearly approves the plan in chat, call record_plan_approval. Tool use remains governed by Den policy and ACP client approval."
        ]
    }))
}

pub async fn cancel_plan_mode(
    ops: &impl PlanModeOps,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, DenError> {
    let args: PlanModeCancelArguments = serde_json::from_value(arguments)?;
    let acp_session_id = require_acp_session(context, "plan mode")?;
    let view = ops.cancel(context, &acp_session_id, args.plan_mode_id).await?;
    Ok(json!({
        "domain": "workplan",
        "workplan": view.workplan,
        "plan_mode": view.plan_mode,
        "workflow_state": view.workflow_state,
        "mode_update": "ask"
    }))
}
