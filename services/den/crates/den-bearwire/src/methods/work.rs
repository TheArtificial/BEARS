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
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use den_docket::work_runs;
use den_http::errors::CustomError;
use den_service::DenState;

use crate::auth::authenticated_bear;
use crate::methods::parse_params;

#[derive(Deserialize)]
struct WorkCheckoutRequest {
    session_id: String,
    work_order_id: Uuid,
}

pub(crate) async fn work_checkout_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (_user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: WorkCheckoutRequest = parse_params(params)?;

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

    Ok(json!({
        "ok": true,
        "work_run_id": checkout.run.id,
        "job_id": checkout.run.job_id,
        "task_title": checkout.task_title,
        "attempt": checkout.run.attempt,
        "prompt": checkout.prompt,
        "permission_mode": "workspace_write",
        // Deadline is enforced by the sandbox provider + armature env; no
        // per-run override is stored yet.
        "deadline_secs": Value::Null,
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
