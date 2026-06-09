use crate::{
    core::runtime_contracts::runtime_error_is_conflict_pending_approval,
    errors::CustomError,
};

pub(crate) fn looks_like_runtime_waiting_for_approval_error(err: &CustomError) -> bool {
    runtime_error_is_conflict_pending_approval(err)
}

pub(crate) fn looks_like_runtime_no_active_runs_error(err: &CustomError) -> bool {
    crate::core::runtime_contracts::runtime_error_is_no_active_runs_cancel(err)
}

pub(crate) async fn cancel_runtime_runs_by_id_or_skip(
    state: &crate::api::service::ApiState,
    pair_agent_id: &str,
    run_ids: &[String],
    reason: &str,
) -> serde_json::Value {
    if state.config.uses_native_agent_runtime() {
        let _ = (pair_agent_id, reason);
        return serde_json::json!({
            "ok": true,
            "skipped": run_ids.is_empty(),
            "attempted": !run_ids.is_empty(),
            "run_ids": run_ids,
            "result": "native:in-process cancel (no external run ids)",
        });
    }
    cancel_letta_runtime_runs_by_id_or_skip(state.letta.as_ref(), pair_agent_id, run_ids, reason)
        .await
}

async fn cancel_letta_runtime_runs_by_id_or_skip(
    letta: &crate::core::letta::LettaClient,
    pair_agent_id: &str,
    run_ids: &[String],
    reason: &str,
) -> serde_json::Value {
    use crate::core::{
        acp_turn_runner_letta::LettaRuntimeCancellationBackend,
        runtime_contracts::{CancelTurnRequest, RoleRuntimeBinding, RuntimeCancellationBackend},
    };

    let request = CancelTurnRequest {
        conversation: crate::core::runtime_contracts::RuntimeConversationRef {
            id: "unknown-conversation".to_string(),
        },
        turn: None,
        reason: Some(reason.to_string()),
        binding: Some(RoleRuntimeBinding {
            binding_id: pair_agent_id.to_string(),
            compatibility_backend: Some("runtime:letta".to_string()),
        }),
        run_ids: run_ids.to_vec(),
    };
    match LettaRuntimeCancellationBackend::new(letta)
        .cancel_turn(request)
        .await
    {
        Ok(result) => serde_json::json!({
            "ok": true,
            "skipped": result.skipped,
            "attempted": !result.skipped,
            "run_ids": run_ids,
            "result": result.detail,
        }),
        Err(err) if looks_like_runtime_no_active_runs_error(&err) => serde_json::json!({
            "ok": true,
            "skipped": false,
            "attempted": true,
            "run_ids": run_ids,
            "result": "no_active_runs",
        }),
        Err(err) => serde_json::json!({
            "ok": false,
            "skipped": false,
            "attempted": true,
            "run_ids": run_ids,
            "error": err.to_string(),
        }),
    }
}
