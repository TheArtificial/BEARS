//! `work.*` methods: the in-sandbox headless armature's handshake with its
//! dispatched work run.
//!
//! `work.checkout` binds the armature's BearWire session to the
//! `bear_work_runs` row it was launched for, opens the Docket execution
//! session whose job/task focus satisfies the Work-stance gate, and returns
//! the prompt built Den-side from the durable task definition.
//! `work.report` stores the armature's advisory summary; the authoritative
//! outcome is the run-completion hook plus Docket task state.

use axum::http::HeaderMap;
use bearwire_protocol::compatibility::{CompatibilityManifest, REQUIRED_WORK_CAPABILITIES};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use den_docket::{
    work_runs, DocketCheckpointDirectiveAcknowledge, DocketService, DocketWorkBoundaryCheck,
    PgDocketService,
};
use den_http::errors::CustomError;
use den_service::{
    bears::{render_turn_fragment, repository_prompt_fragment_registry},
    DenState,
};

use crate::auth::authenticated_bear;
use crate::methods::parse_params;

#[derive(Deserialize)]
struct WorkCheckoutRequest {
    session_id: String,
    work_order_id: Uuid,
    compatibility: CompatibilityManifest,
}

pub(crate) async fn work_checkout_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (_user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: WorkCheckoutRequest = parse_params(params)?;

    let missing = request
        .compatibility
        .missing(REQUIRED_WORK_CAPABILITIES)
        .collect::<Vec<_>>();
    if request.compatibility.protocol != 1 || !missing.is_empty() {
        return Err(CustomError::ValidationError(format!(
            "incompatible sandbox armature: protocol={}, required_protocol=1, missing_capabilities={missing:?}",
            request.compatibility.protocol
        )));
    }

    // Compatibility must be checked before checkout: checkout binds the
    // session and mutates durable run/task state.
    let checkout = work_runs::checkout_work_run_for_session(
        &state.sqlx_pool,
        request.work_order_id,
        bear.id,
        &request.session_id,
    )
    .await?;

    tracing::info!(
        work_run_id = %checkout.run.id,
        job_id = %checkout.run.job_id,
        session_id = %request.session_id,
        bear_slug = %bear.slug,
        "work.checkout bound armature session to work run"
    );

    let prompt = match checkout.prompt_context.as_ref() {
        Some(prompt_context) => {
            let prompt_registry = repository_prompt_fragment_registry()?;
            let prompt_fragment = prompt_registry.require("runtime_work_checkout")?;
            render_turn_fragment(prompt_fragment, &json!({ "work": prompt_context }))?
        }
        None => String::new(),
    };

    let authorized = matches!(
        &checkout.gate,
        den_docket::DocketExecutionGate::Allowed { .. }
    );
    Ok(json!({
        "ok": authorized,
        "work_run_id": checkout.run.id,
        "job_id": checkout.run.job_id,
        "task_title": checkout.task_title,
        "gate": checkout.gate,
        "attempt": checkout.run.attempt,
        "execution_attempt_id": checkout.execution_attempt.as_ref().map(|attempt| attempt.id),
        "execution_attempt_fence_epoch": checkout.execution_attempt.as_ref().map(|attempt| attempt.fence_epoch),
        "prompt": prompt,
        "permission_mode": if authorized { "workspace_write" } else { "none" },
        // Deadline is enforced by the sandbox provider + armature env; no
        // per-run override is stored yet.
        "deadline_secs": Value::Null,
    }))
}

pub(crate) async fn work_boundary_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (_user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: WorkBoundaryRequest = parse_params(params)?;
    let gate = PgDocketService::from_pool(&state.sqlx_pool)
        .check_work_boundary(DocketWorkBoundaryCheck {
            bear_id: bear.id,
            attempt_id: request.execution_attempt_id,
            fence_epoch: request.fence_epoch,
            boundary_key: request.boundary_key,
        })
        .await?;
    Ok(
        json!({ "ok": matches!(gate, den_docket::DocketExecutionGate::Allowed { .. }), "gate": gate }),
    )
}

#[derive(Deserialize)]
struct WorkBoundaryRequest {
    execution_attempt_id: Uuid,
    fence_epoch: i64,
    boundary_key: Uuid,
}

#[derive(Deserialize)]
struct WorkAcknowledgeCheckpointRequest {
    directive_id: Uuid,
    execution_attempt_id: Uuid,
    fence_epoch: i64,
    checkpoint_artifact_ref: String,
}

pub(crate) async fn work_acknowledge_checkpoint_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (_user_id, _bear) = authenticated_bear(state, headers, params).await?;
    let request: WorkAcknowledgeCheckpointRequest = parse_params(params)?;
    let directive = PgDocketService::from_pool(&state.sqlx_pool)
        .acknowledge_checkpoint_directive(DocketCheckpointDirectiveAcknowledge {
            bear_id: _bear.id,
            directive_id: request.directive_id,
            execution_attempt_id: request.execution_attempt_id,
            fence_epoch: request.fence_epoch,
            artifact_ref: request.checkpoint_artifact_ref,
        })
        .await?;

    Ok(json!({
        "ok": true,
        "directive_id": directive.id,
        "state": directive.state,
        "acknowledged_artifact_ref": directive.acknowledged_artifact_ref,
    }))
}
#[derive(Deserialize)]
struct WorkReportRequest {
    #[allow(dead_code)]
    session_id: String,
    work_order_id: Uuid,
    #[serde(default)]
    status_hint: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

pub(crate) async fn work_report_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (_user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: WorkReportRequest = parse_params(params)?;

    work_runs::record_work_run_report(
        &state.sqlx_pool,
        request.work_order_id,
        bear.id,
        request.status_hint.as_deref().unwrap_or("unknown"),
        request.summary.as_deref().unwrap_or(""),
    )
    .await?;

    Ok(json!({ "ok": true }))
}
