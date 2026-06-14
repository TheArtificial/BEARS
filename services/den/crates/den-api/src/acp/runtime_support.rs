use crate::errors::DenError;

pub(crate) fn looks_like_runtime_waiting_for_approval_error(err: &DenError) -> bool {
    den_runtime::runtime_contracts::runtime_error_is_conflict_pending_approval(err)
}

pub(crate) async fn cancel_runtime_runs_by_id_or_skip(
    _state: &crate::api::service::ApiState,
    pair_agent_id: &str,
    run_ids: &[String],
    reason: &str,
) -> serde_json::Value {
    let _ = (pair_agent_id, reason);
    serde_json::json!({
        "ok": true,
        "skipped": run_ids.is_empty(),
        "attempted": !run_ids.is_empty(),
        "run_ids": run_ids,
        "result": "native:in-process cancel (no external run ids)",
    })
}
