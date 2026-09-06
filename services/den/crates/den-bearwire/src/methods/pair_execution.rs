use bearwire_protocol::wire::BearWireEvent;
use den_docket::{DocketService, PgDocketService};
use den_http::errors::CustomError;
use den_runtime::{bearwire_events, current_task::select_pair_current_task};
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
    with_execution_lock(state, bear.id, client_session_id, || async {
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
    .await
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
    let session = client_sessions::find_for_user_bear_session_id(
        &state.sqlx_pool,
        user_id,
        bear.id,
        client_session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("client session not found".to_string()))?;

    PgDocketService::from_pool(&state.sqlx_pool)
        .attach_task_to_pair_session(bear.id, task_id, session.id)
        .await?;
    select_pair_current_task(
        &state.sqlx_pool,
        user_id,
        bear.id,
        client_session_id,
        Some(task_id),
    )
    .await?;

    let started = start_pair_current_task(state, user_id, bear.clone(), client_session_id).await?;
    let execution = reduce_start_result(client_session_id, task_id, started)?;
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
