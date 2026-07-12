use std::{collections::BTreeMap, time::Duration};

use den_core::DenError;
use den_runtime::{turn_obligations, turn_runs};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::methods::run::persist_run_failed;

const DEFAULT_EXPIRY_BATCH_LIMIT: i64 = 1_000;

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

    let mut by_run = BTreeMap::<String, Vec<Value>>::new();
    for obligation in expired {
        by_run
            .entry(obligation.run_id.clone())
            .or_default()
            .push(json!({
                "obligation_id": obligation.id,
                "kind": obligation.kind,
                "expected_responder_action": obligation.expected_responder_action,
                "tool_call_id": obligation.tool_call_id,
                "permission_id": obligation.permission_id,
                "timeout_ms": obligation.timeout_ms(),
                "created_at": obligation.created_at,
                "expires_at": obligation.expires_at(),
            }));
    }

    let expired_run_count = by_run.len();
    for (run_id, expired_obligations) in by_run {
        let Some(run) = turn_runs::get_run(pool, &run_id).await? else {
            tracing::warn!(
                run_id,
                expired_obligation_count = expired_obligations.len(),
                "expired BearWire client obligations reference a missing run"
            );
            continue;
        };
        persist_run_failed(
            pool,
            &run.session_id,
            &run.run_id,
            run.bear_id,
            run.user_id,
            "client_obligation_timeout",
            "A required client obligation timed out before the armature/client responded."
                .to_string(),
            Some(json!({
                "expired_obligations": expired_obligations,
                "source": "bearwire_client_obligation_expiry_loop",
            })),
        )
        .await;
    }

    Ok(expired_run_count)
}
