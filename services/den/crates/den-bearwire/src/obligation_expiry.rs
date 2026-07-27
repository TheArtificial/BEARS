use std::{collections::BTreeMap, time::Duration};

use den_core::{client_tools::ClientToolName, DenError};
use den_runtime::{turn_obligations, turn_runs};
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
    pool: sqlx::PgPool,
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
                if let Err(err) = expire_client_obligations_once(&pool, DEFAULT_EXPIRY_BATCH_LIMIT).await {
                    tracing::warn!(error = %err, "BearWire client-obligation expiry sweep failed");
                }
            }
        }
    }
}

pub async fn expire_client_obligations_once(
    pool: &sqlx::PgPool,
    limit: i64,
) -> Result<usize, DenError> {
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
                "A command exceeded its response deadline, and Den could not confirm whether it completed or is still running. Do not retry it until you inspect the client and workspace state.".to_string(),
                Some(json!({
                    "status": "outcome_unknown",
                    "automatic_retry_allowed": false,
                    "next_action": "reconnect_and_inspect",
                })),
            )
        } else {
            (
                RunFailureReason::ClientObligationTimeout,
                "The connected client did not respond before the required step timed out. Reconnect the client and start a new turn to retry.".to_string(),
                None,
            )
        };
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
