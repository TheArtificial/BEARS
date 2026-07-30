use std::{collections::BTreeMap, time::Duration};

use den_core::{client_tools::ClientToolName, DenError};
use den_runtime::{turn_obligations, turn_runs};
use den_service::DenState;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::methods::run::{persist_run_failed, RunFailureReason};

const DEFAULT_EXPIRY_BATCH_LIMIT: i64 = 1_000;

fn is_command_tool_result(obligation: &turn_obligations::TurnObligationRow) -> bool {
    if obligation.expected_responder_action != "tool_result" {
        return false;
    }
    let Some(tool_name) = obligation
        .request_payload
        .get("tool_name")
        .and_then(Value::as_str)
    else {
        return false;
    };
    matches!(
        ClientToolName::from_provider_alias(tool_name),
        Some(
            ClientToolName::RunCommand
                | ClientToolName::ProcessRun
                | ClientToolName::TerminalRunCommand
        )
    )
}

pub async fn run_client_obligation_expiry_loop(
    state: DenState,
    token: CancellationToken,
    interval: Duration,
) -> Result<(), DenError> {
    tracing::info!(
        interval_ms = interval.as_millis(),
        "BearWire client-obligation expiry loop enabled"
    );
    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("BearWire client-obligation expiry loop stopped");
                return Ok(());
            }
            () = tokio::time::sleep(interval) => {
                if let Err(err) = expire_client_obligations_once(&state, DEFAULT_EXPIRY_BATCH_LIMIT).await {
                    tracing::warn!(error = %err, "BearWire client-obligation expiry sweep failed");
                }
            }
        }
    }
}

pub async fn expire_client_obligations_once(
    state: &DenState,
    limit: i64,
) -> Result<usize, DenError> {
    let pool = &state.sqlx_pool;
    let expired = turn_obligations::expire_open_client_obligations(pool, limit).await?;
    if expired.is_empty() {
        return Ok(0);
    }

    let mut by_run = BTreeMap::<String, (Vec<Value>, bool)>::new();
    for obligation in expired {
        let command_outcome_unknown = is_command_tool_result(&obligation);
        let entry = by_run
            .entry(obligation.run_id.clone())
            .or_insert_with(|| (Vec::new(), false));
        entry.0.push(json!({
            "obligation_id": obligation.id,
            "kind": obligation.kind,
            "expected_responder_action": obligation.expected_responder_action,
            "tool_call_id": obligation.tool_call_id,
            "tool_name": obligation.request_payload.get("tool_name"),
            "permission_id": obligation.permission_id,
            "timeout_ms": obligation.timeout_ms(),
            "created_at": obligation.created_at,
            "expires_at": obligation.expires_at(),
        }));
        entry.1 |= command_outcome_unknown;
    }

    let expired_run_count = by_run.len();
    for (run_id, (expired_obligations, command_outcome_unknown)) in by_run {
        let Some(run) = turn_runs::get_run(pool, &run_id).await? else {
            tracing::warn!(
                run_id,
                expired_obligation_count = expired_obligations.len(),
                "expired BearWire client obligations reference a missing run"
            );
            continue;
        };
        let (reason, message, recovery) = if command_outcome_unknown {
            (
                RunFailureReason::CommandOutcomeUnknown,
                "Command result couldn't be confirmed. The connection ended before Builder Bear could report whether the command completed. To avoid duplicate changes, the command was not retried automatically.".to_string(),
                Some(json!({
                    "status": "outcome_unknown",
                    "automatic_retry_allowed": false,
                    "next_action": "run_state",
                    "next_action_params": { "run_id": run_id },
                })),
            )
        } else {
            let timed_out_permissions = expired_obligations
                .iter()
                .filter(|obligation| {
                    obligation
                        .get("expected_responder_action")
                        .and_then(Value::as_str)
                        == Some("permission_decision")
                })
                .filter_map(|obligation| obligation.get("permission_id").and_then(Value::as_str))
                .collect::<Vec<_>>();
            let detail = match timed_out_permissions.as_slice() {
                [permission_id] => {
                    format!("Permission request {permission_id} timed out waiting for a client response.")
                }
                [] => "The connected client did not respond before the required step timed out."
                    .to_string(),
                permission_ids => format!(
                    "Permission requests {} timed out waiting for client responses.",
                    permission_ids.join(", ")
                ),
            };
            (
                RunFailureReason::ClientObligationTimeout,
                format!("{detail} Reconnect the client and start a new turn to retry."),
                None,
            )
        };
        // The durable failure alone does not wake an ACP continuation already
        // blocked on this client response.
        state.turn_cancellations.cancel_session(&run.session_id);
        state.tool_turns.cancel_active_turn(&run.session_id);
        persist_run_failed(
            pool,
            &run.session_id,
            &run.run_id,
            run.bear_id,
            run.user_id,
            reason,
            message,
            Some(json!({
                "expired_obligations": expired_obligations,
                "source": "bearwire_client_obligation_expiry_loop",
                "recovery": recovery,
            })),
        )
        .await;
    }

    Ok(expired_run_count)
}
