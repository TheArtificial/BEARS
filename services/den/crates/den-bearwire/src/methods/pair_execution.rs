use bearwire_protocol::wire::BearWireEvent;
use den_docket::DocketService;
use den_http::errors::CustomError;
use den_runtime::bearwire_events;
use den_service::{bears::Bear, client_sessions, DenState};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use super::session::{start_pair_current_task, PairTaskStartResult};

/// Authoritative result of asking Den to start or reconcile Docket-owned Pair execution.
///
/// Entry points such as `/focus` must consume this result rather than rebuilding loop-control
/// state from stance- or client-specific response fields.
#[derive(Debug, Clone, Serialize)]
pub struct DocketPairExecution {
    pub session_id: String,
    pub task_id: Uuid,
    pub run_id: String,
    pub run_state: String,
    pub attempt_id: Uuid,
    pub attempt_state: String,
    pub launch_state: String,
    pub fence_epoch: i64,
}

impl DocketPairExecution {
    pub fn is_live(&self) -> bool {
        self.attempt_state == "running"
            && matches!(self.run_state.as_str(), "running" | "waiting_for_client")
            && matches!(self.launch_state.as_str(), "started" | "already_running")
    }
}

/// Start or reconcile the selected Docket task against a Pair execution owner.
///
/// This is the Den-side command boundary. Docket RPCs, `/focus`, ACP, and other clients should
/// not reproduce its attach/select/start/reduce sequence.
pub async fn start_or_reconcile_docket_pair_execution(
    state: &DenState,
    user_id: i32,
    bear: Bear,
    client_session_id: &str,
    task_id: Uuid,
) -> Result<DocketPairExecution, CustomError> {
    with_execution_lock(state, bear.id, client_session_id, || async {
        start_or_reconcile_locked(state, user_id, bear, client_session_id, task_id).await
    })
    .await
}

/// Starts focused execution for the session's already-selected task. This keeps the
/// Pair-facing focus RPC on the same serialized command boundary as Docket `/focus`.
pub async fn start_selected_docket_pair_execution(
    state: &DenState,
    user_id: i32,
    bear: Bear,
    client_session_id: &str,
) -> Result<DocketPairExecution, CustomError> {
    let bear_id = bear.id;
    tracing::info!(
        event = "docket_pair_focus_requested",
        bear_id = %bear_id,
        user_id,
        client_session_id,
        "received Pair focus request"
    );
    let result = with_execution_lock(state, bear_id, client_session_id, || async {
        let session = client_sessions::find_for_user_bear_session_id(
            &state.sqlx_pool,
            user_id,
            bear.id,
            client_session_id,
        )
        .await?
        .ok_or_else(|| CustomError::NotFound("client session not found".to_string()))?;
        let task_id = session.current_task_id.ok_or_else(|| {
            CustomError::ValidationError(
                "no current Pair task is selected for this session".to_string(),
            )
        })?;
        start_or_reconcile_locked(state, user_id, bear, client_session_id, task_id).await
    })
    .await;
    match &result {
        Ok(execution) => tracing::info!(
            event = "docket_pair_focus_established",
            bear_id = %bear_id,
            user_id,
            client_session_id,
            task_id = %execution.task_id,
            run_id = %execution.run_id,
            attempt_id = %execution.attempt_id,
            launch_state = %execution.launch_state,
            fence_epoch = execution.fence_epoch,
            "Pair focus established execution control"
        ),
        Err(error) => tracing::warn!(
            event = "docket_pair_focus_failed",
            bear_id = %bear_id,
            user_id,
            client_session_id,
            error = %error,
            "Pair focus did not establish execution control"
        ),
    }
    result
}

async fn with_execution_lock<T, F, Fut>(
    state: &DenState,
    bear_id: Uuid,
    client_session_id: &str,
    operation: F,
) -> Result<T, CustomError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, CustomError>>,
{
    let lock_key = format!("docket-pair-execution:{bear_id}:{client_session_id}");
    let mut lock_transaction = state.sqlx_pool.begin().await?;
    sqlx::query!(
        r#"SELECT pg_advisory_xact_lock(hashtextextended($1, 0)) AS "locked!""#,
        lock_key
    )
    .fetch_one(&mut *lock_transaction)
    .await?;
    // ponytail: PostgreSQL advisory locking is global per database; if command volume
    // becomes material, replace it with a persisted command queue/lease.
    let result = operation().await;
    let release_result = lock_transaction.commit().await;
    match (result, release_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(CustomError::System(format!(
            "release Docket Pair execution command lock failed: {err}"
        ))),
    }
}

async fn start_or_reconcile_locked(
    state: &DenState,
    user_id: i32,
    bear: Bear,
    client_session_id: &str,
    task_id: Uuid,
) -> Result<DocketPairExecution, CustomError> {
    if let Some(existing) = live_pair_execution(state, bear.id, client_session_id, task_id).await? {
        return Ok(existing);
    }
    let session = client_sessions::find_for_user_bear_session_id(
        &state.sqlx_pool,
        user_id,
        bear.id,
        client_session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("client session not found".to_string()))?;

    activate_pair_task(
        state,
        user_id,
        bear.id,
        client_session_id,
        session.id,
        task_id,
    )
    .await?;
    tracing::info!(
        event = "docket_pair_focus_task_activated",
        bear_id = %bear.id,
        user_id,
        client_session_id,
        task_id = %task_id,
        "persisted Pair task attachment for focus"
    );

    let started = start_pair_current_task(state, user_id, bear.clone(), client_session_id).await?;
    let execution = reduce_start_result(client_session_id, task_id, started)?;
    tracing::info!(
        event = "docket_pair_focus_start_resolved",
        bear_id = %bear.id,
        user_id,
        client_session_id,
        task_id = %execution.task_id,
        run_id = %execution.run_id,
        attempt_id = %execution.attempt_id,
        attempt_state = %execution.attempt_state,
        run_state = %execution.run_state,
        launch_state = %execution.launch_state,
        fence_epoch = execution.fence_epoch,
        "resolved Pair focus execution start"
    );
    if !execution.is_live() {
        return Err(CustomError::System(format!(
            "Docket Pair execution is not live (attempt_state={}, run_state={}, launch_state={})",
            execution.attempt_state, execution.run_state, execution.launch_state
        )));
    }

    let mut event = BearWireEvent::ephemeral(
        "docket.execution.started",
        json!({
            "attempt_id": execution.attempt_id,
            "task_id": execution.task_id,
            "run_id": execution.run_id,
            "binding": { "kind": "client_session", "id": client_session_id },
            "host": { "kind": "pair", "run_id": execution.run_id },
            "attempt_state": execution.attempt_state,
            "launch_state": execution.launch_state,
            "fence_epoch": execution.fence_epoch,
            "task_selection_preserved": true,
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(client_session_id.to_string());
    event.run_id = Some(execution.run_id.clone());
    if let Err(err) = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        client_session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await
    {
        // The execution aggregate above is authoritative. A transcript projection
        // failure must not turn an already-started execution into an ambiguous RPC
        // failure; the durable run/attempt state remains available for reconciliation.
        tracing::warn!(
            error = %err,
            session_id = client_session_id,
            run_id = %execution.run_id,
            attempt_id = %execution.attempt_id,
            "failed to project Docket execution start"
        );
    }

    Ok(execution)
}

/// Commits the durable part of `/focus` in one transaction. Starting the loop
/// remains outside it: startup is reconciled idempotently after interruption,
/// while an attachment without its selected task is not a valid focus state.
async fn activate_pair_task(
    state: &DenState,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    session_id: Uuid,
    task_id: Uuid,
) -> Result<(), CustomError> {
    let mut tx = state.sqlx_pool.begin().await?;
    let session_exists = sqlx::query(
        "SELECT 1 FROM client_sessions WHERE id = $1 AND user_id = $2 AND bear_id = $3 AND client_session_id = $4 FOR UPDATE",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(bear_id)
    .bind(client_session_id)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if !session_exists {
        return Err(CustomError::NotFound(
            "client session not found".to_string(),
        ));
    }

    let attached = sqlx::query(
        r#"
        INSERT INTO bear_pair_task_attachments (task_id, session_id)
        SELECT id, $3 FROM bear_tasks
        WHERE id = $2 AND bear_id = $1 AND settled_by_entry_id IS NULL
          AND (job_id IS NOT NULL OR EXISTS (
            SELECT 1 FROM bear_pair_task_attachments existing
            WHERE existing.task_id = bear_tasks.id
              AND existing.session_id = $3 AND existing.released_at IS NULL
          ))
        ON CONFLICT (task_id) DO UPDATE
        SET session_id = EXCLUDED.session_id, attached_at = NOW(), released_at = NULL
        WHERE bear_pair_task_attachments.released_at IS NOT NULL
           OR bear_pair_task_attachments.session_id = EXCLUDED.session_id
        "#,
    )
    .bind(bear_id)
    .bind(task_id)
    .bind(session_id)
    .execute(&mut *tx)
    .await?;
    if attached.rows_affected() == 0 {
        return Err(CustomError::ValidationError(
            "task is not an unclaimed durable task available to this Pair session".to_string(),
        ));
    }

    sqlx::query(
        "UPDATE client_sessions SET current_task_id = $2, updated_at = NOW() WHERE id = $1",
    )
    .bind(session_id)
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::debug!(%bear_id, %session_id, %task_id, "activated Pair task for focused execution");
    Ok(())
}

async fn live_pair_execution(
    state: &DenState,
    bear_id: Uuid,
    client_session_id: &str,
    task_id: Uuid,
) -> Result<Option<DocketPairExecution>, CustomError> {
    let Some(run) =
        den_runtime::turn_runs::active_run_for_session(&state.sqlx_pool, client_session_id)
            .await?
            .filter(|run| {
                run.bear_id == bear_id
                    && matches!(run.state.as_str(), "running" | "waiting_for_client")
            })
    else {
        return Ok(None);
    };
    let Some(attempt) = den_docket::PgDocketService::from_pool(&state.sqlx_pool)
        .get_live_pair_execution_attempt(bear_id, task_id, client_session_id, &run.run_id)
        .await?
    else {
        return Ok(None);
    };
    let controller_is_live = state
        .turn_cancellations
        .active_for_session(client_session_id)
        .is_some_and(|active| active.run_ids.iter().any(|id| id == &run.run_id));
    if !controller_is_live {
        return Ok(None);
    }
    Ok(Some(DocketPairExecution {
        session_id: client_session_id.to_owned(),
        task_id,
        run_id: run.run_id,
        run_state: run.state,
        attempt_id: attempt.id,
        attempt_state: "running".to_owned(),
        launch_state: "already_running".to_owned(),
        fence_epoch: attempt.fence_epoch,
    }))
}

fn reduce_start_result(
    session_id: &str,
    task_id: Uuid,
    result: PairTaskStartResult,
) -> Result<DocketPairExecution, CustomError> {
    if result.task_id != task_id || result.session_id != session_id {
        return Err(CustomError::System(
            "Pair task start returned mismatched task or session authority".to_string(),
        ));
    }

    Ok(DocketPairExecution {
        session_id: result.session_id,
        task_id: result.task_id,
        run_id: result.run_id,
        run_state: result.state,
        attempt_id: result.execution_attempt_id,
        attempt_state: result.execution_attempt_state,
        launch_state: result.launch_state,
        fence_epoch: result.fence_epoch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_execution_requires_correlated_running_states() {
        let execution = DocketPairExecution {
            session_id: "session".into(),
            task_id: Uuid::nil(),
            run_id: "run".into(),
            run_state: "running".into(),
            attempt_id: Uuid::nil(),
            attempt_state: "running".into(),
            launch_state: "started".into(),
            fence_epoch: 1,
        };
        assert!(execution.is_live());
        assert!(
            DocketPairExecution {
                launch_state: "missing".into(),
                ..execution
            }
            .is_live()
                == false
        );
    }
}
