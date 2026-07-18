use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

use crate::turn_obligations::TurnObligationState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRunState {
    Accepted,
    Running,
    WaitingForClient,
    WaitingForToolResult,
    WaitingForPermission,
    Continuing,
    Completed,
    Failed,
    Cancelled,
}

impl TurnRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::WaitingForClient => "waiting_for_client",
            Self::WaitingForToolResult => "waiting_for_tool_result",
            Self::WaitingForPermission => "waiting_for_permission",
            Self::Continuing => "continuing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn try_from_storage(value: &str) -> Result<Self, DenError> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "running" => Ok(Self::Running),
            "waiting_for_client" => Ok(Self::WaitingForClient),
            "waiting_for_tool_result" => Ok(Self::WaitingForToolResult),
            "waiting_for_permission" => Ok(Self::WaitingForPermission),
            "continuing" => Ok(Self::Continuing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(DenError::ValidationError(format!(
                "unsupported turn run state: {other}"
            ))),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn allows_open_obligation(self) -> bool {
        !self.is_terminal()
    }

    fn terminal_obligation_state(self) -> Option<TurnObligationState> {
        match self {
            Self::Completed => Some(TurnObligationState::Continued),
            Self::Failed => Some(TurnObligationState::Failed),
            Self::Cancelled => Some(TurnObligationState::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnRunRow {
    pub id: Uuid,
    pub run_id: String,
    pub session_id: String,
    pub bear_id: Uuid,
    pub user_id: i32,
    pub state: String,
    pub terminal_reason: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
}

impl TurnRunRow {
    pub fn state_value(&self) -> Result<TurnRunState, DenError> {
        TurnRunState::try_from_storage(&self.state)
    }
}

fn row_to_run(row: sqlx::postgres::PgRow) -> TurnRunRow {
    TurnRunRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        session_id: row.get("session_id"),
        bear_id: row.get("bear_id"),
        user_id: row.get("user_id"),
        state: row.get("state"),
        terminal_reason: row.get("terminal_reason"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
    }
}

const RUN_RETURNING: &str = r"
    id, run_id, session_id, bear_id, user_id, state,
    terminal_reason, created_at, updated_at, completed_at
";
const ACTIVE_RUN_STATES_SQL: &str = "'accepted','running','waiting_for_client','waiting_for_tool_result','waiting_for_permission','continuing'";

pub async fn create_run(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    bear_id: Uuid,
    user_id: i32,
) -> Result<TurnRunRow, DenError> {
    let row = sqlx::query(&format!(
        r"
        INSERT INTO turn_runs (run_id, session_id, bear_id, user_id, state)
        VALUES ($1, $2, $3, $4, 'accepted')
        RETURNING {RUN_RETURNING}
        "
    ))
    .bind(run_id)
    .bind(session_id)
    .bind(bear_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(row_to_run(row))
}

pub async fn get_run(pool: &PgPool, run_id: &str) -> Result<Option<TurnRunRow>, DenError> {
    let row = sqlx::query(&format!(
        r"
        SELECT {RUN_RETURNING}
        FROM turn_runs
        WHERE run_id = $1
        "
    ))
    .bind(run_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}

pub async fn active_run_for_session(
    pool: &PgPool,
    session_id: &str,
) -> Result<Option<TurnRunRow>, DenError> {
    let row = sqlx::query(&format!(
        r"
        SELECT {RUN_RETURNING}
        FROM turn_runs
        WHERE session_id = $1
          AND state IN ({ACTIVE_RUN_STATES_SQL})
        ORDER BY created_at DESC
        LIMIT 1
        "
    ))
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}

pub async fn supersede_active_run_for_session(
    pool: &PgPool,
    session_id: &str,
    bear_id: Uuid,
    user_id: i32,
    reason: &str,
) -> Result<Option<TurnRunRow>, DenError> {
    let row = sqlx::query(&format!(
        r"
        UPDATE turn_runs
        SET state = 'failed', terminal_reason = $4, completed_at = NOW(), updated_at = NOW()
        WHERE id = (
            SELECT id
            FROM turn_runs
            WHERE session_id = $1 AND bear_id = $2 AND user_id = $3
              AND state IN ({ACTIVE_RUN_STATES_SQL})
            ORDER BY created_at DESC
            LIMIT 1
        )
        RETURNING {RUN_RETURNING}
        "
    ))
    .bind(session_id)
    .bind(bear_id)
    .bind(user_id)
    .bind(reason)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnObligationResultRow {
    pub id: Uuid,
    pub run_id: String,
    pub obligation_kind: String,
    pub obligation_id: String,
    pub result_hash: String,
    pub payload_json: serde_json::Value,
    pub turn_step_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TurnObligationResultRecord {
    Inserted { row: TurnObligationResultRow },
    DuplicateIdentical { row: TurnObligationResultRow },
    DuplicateConflict { existing_hash: String },
}

fn result_hash(payload: &serde_json::Value) -> Result<String, DenError> {
    let bytes = serde_json::to_vec(payload).map_err(|err| {
        DenError::System(format!("serialize BearWire client result failed: {err}"))
    })?;
    let digest = Sha256::digest(&bytes);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
}

fn row_to_client_result(row: sqlx::postgres::PgRow) -> TurnObligationResultRow {
    TurnObligationResultRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        obligation_kind: row.get("obligation_kind"),
        obligation_id: row.get("obligation_id"),
        result_hash: row.get("result_hash"),
        payload_json: row.get("payload_json"),
        turn_step_id: row.try_get("turn_step_id").ok(),
        created_at: row.get("created_at"),
    }
}

pub async fn existing_client_result_for_payload(
    pool: &PgPool,
    run_id: &str,
    obligation_kind: &str,
    obligation_id: &str,
    payload_json: &serde_json::Value,
) -> Result<Option<TurnObligationResultRecord>, DenError> {
    let Some(existing) = sqlx::query(
        r"
        SELECT id, run_id, obligation_kind, obligation_id, result_hash, payload_json, turn_step_id, created_at
        FROM turn_obligation_results
        WHERE run_id = $1 AND obligation_kind = $2 AND obligation_id = $3
        ",
    )
    .bind(run_id)
    .bind(obligation_kind)
    .bind(obligation_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let row = row_to_client_result(existing);
    if row.result_hash == result_hash(payload_json)? {
        Ok(Some(TurnObligationResultRecord::DuplicateIdentical { row }))
    } else {
        Ok(Some(TurnObligationResultRecord::DuplicateConflict {
            existing_hash: row.result_hash,
        }))
    }
}

pub async fn record_client_result(
    pool: &PgPool,
    run_id: &str,
    obligation_kind: &str,
    obligation_id: &str,
    payload_json: serde_json::Value,
) -> Result<TurnObligationResultRecord, DenError> {
    record_client_result_for_step(
        pool,
        run_id,
        None,
        obligation_kind,
        obligation_id,
        payload_json,
    )
    .await
}

pub async fn record_client_result_for_step(
    pool: &PgPool,
    run_id: &str,
    turn_step_id: Option<Uuid>,
    obligation_kind: &str,
    obligation_id: &str,
    payload_json: serde_json::Value,
) -> Result<TurnObligationResultRecord, DenError> {
    let hash = result_hash(&payload_json)?;
    let inserted = sqlx::query(
        r"
        INSERT INTO turn_obligation_results (
            run_id, turn_step_id, obligation_kind, obligation_id, result_hash, payload_json
        ) VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (run_id, obligation_kind, obligation_id) DO NOTHING
        RETURNING id, run_id, obligation_kind, obligation_id, result_hash, payload_json, turn_step_id, created_at
        ",
    )
    .bind(run_id)
    .bind(turn_step_id)
    .bind(obligation_kind)
    .bind(obligation_id)
    .bind(&hash)
    .bind(&payload_json)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = inserted {
        return Ok(TurnObligationResultRecord::Inserted {
            row: row_to_client_result(row),
        });
    }

    let existing = sqlx::query(
        r"
        SELECT id, run_id, obligation_kind, obligation_id, result_hash, payload_json, turn_step_id, created_at
        FROM turn_obligation_results
        WHERE run_id = $1 AND obligation_kind = $2 AND obligation_id = $3
        ",
    )
    .bind(run_id)
    .bind(obligation_kind)
    .bind(obligation_id)
    .fetch_one(pool)
    .await?;
    let row = row_to_client_result(existing);
    if row.result_hash == hash {
        Ok(TurnObligationResultRecord::DuplicateIdentical { row })
    } else {
        Ok(TurnObligationResultRecord::DuplicateConflict {
            existing_hash: row.result_hash,
        })
    }
}

pub async fn client_result_count_for_run_kind(
    pool: &PgPool,
    run_id: &str,
    obligation_kind: &str,
) -> Result<i64, DenError> {
    let count = sqlx::query_scalar::<_, i64>(
        r"
        SELECT COUNT(*)
        FROM turn_obligation_results
        WHERE run_id = $1 AND obligation_kind = $2
        ",
    )
    .bind(run_id)
    .bind(obligation_kind)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinishRunResult {
    pub settled_obligations: u64,
    pub settled_steps: u64,
    pub event_sequence: i64,
}

pub async fn finish_run_with_bearwire_event(
    pool: &PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: Uuid,
    user_id: i32,
    state: TurnRunState,
    terminal_reason: Option<&str>,
    mut event: bearwire_protocol::wire::BearWireEvent,
) -> Result<Option<FinishRunResult>, DenError> {
    if !state.is_terminal() {
        return Err(DenError::ValidationError(format!(
            "finish_run_with_bearwire_event requires terminal state, got {}",
            state.as_str()
        )));
    }
    let obligation_state = state
        .terminal_obligation_state()
        .expect("terminal state has obligation settlement");
    let settlement_state = obligation_state.as_str();
    let mut tx = pool.begin().await?;
    let claimed = sqlx::query(
        r"
        UPDATE turn_runs
        SET state = $2,
            terminal_reason = $3,
            updated_at = NOW(),
            completed_at = COALESCE(completed_at, NOW())
        WHERE run_id = $1
          AND session_id = $4
          AND state NOT IN ('completed','failed','cancelled')
        ",
    )
    .bind(run_id)
    .bind(state.as_str())
    .bind(terminal_reason)
    .bind(session_id)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if !claimed {
        tx.rollback().await?;
        return Ok(None);
    }
    let settled_obligations = sqlx::query(
        r"
        UPDATE turn_obligations
        SET state = $2,
            completed_at = COALESCE(completed_at, NOW()),
            updated_at = NOW()
        WHERE run_id = $1
          AND state IN ('requested','waiting_for_client','result_received')
        ",
    )
    .bind(run_id)
    .bind(settlement_state)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let settled_steps = sqlx::query(
        r"
        UPDATE turn_steps
        SET state = $2,
            closed_at = COALESCE(closed_at, NOW())
        WHERE run_id = $1
          AND state IN ('streaming_model','waiting_for_client','ready_to_continue')
        ",
    )
    .bind(run_id)
    .bind(settlement_state)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    event.data["settled_obligations"] = serde_json::json!(settled_obligations);
    event.data["settled_steps"] = serde_json::json!(settled_steps);
    let terminal_event = crate::bearwire_events::append_bearwire_event_on(
        &mut tx,
        session_id,
        Some(bear_id),
        Some(user_id),
        event,
    )
    .await?;
    tx.commit().await?;
    Ok(Some(FinishRunResult {
        settled_obligations,
        settled_steps,
        event_sequence: terminal_event.sequence_no,
    }))
}

pub async fn transition_run(
    pool: &PgPool,
    run_id: &str,
    state: TurnRunState,
    terminal_reason: Option<&str>,
) -> Result<Option<TurnRunRow>, DenError> {
    if state.is_terminal() {
        return Err(DenError::ValidationError(format!(
            "terminal run state {} must use finish_run_with_bearwire_event",
            state.as_str()
        )));
    }
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        r"
        UPDATE turn_runs
        SET state = $2,
            terminal_reason = $3,
            updated_at = NOW(),
            completed_at = completed_at
        WHERE run_id = $1
          AND state NOT IN ('completed','failed','cancelled')
        RETURNING {RUN_RETURNING}
        "
    ))
    .bind(run_id)
    .bind(state.as_str())
    .bind(terminal_reason)
    .fetch_optional(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(row.map(row_to_run))
}

#[cfg(test)]
mod tests {
    use super::TurnRunState;

    #[test]
    fn terminal_run_with_open_obligation_is_invalid() {
        for terminal in [
            TurnRunState::Completed,
            TurnRunState::Failed,
            TurnRunState::Cancelled,
        ] {
            assert_eq!(
                terminal.terminal_obligation_state().is_some(),
                terminal.is_terminal()
            );
            assert!(!terminal.allows_open_obligation());
        }

        assert!(TurnRunState::WaitingForToolResult.allows_open_obligation());
        assert!(TurnRunState::WaitingForPermission.allows_open_obligation());
    }
}
