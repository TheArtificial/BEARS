use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStepState {
    StreamingModel,
    WaitingForClient,
    ReadyToContinue,
    Continued,
    Failed,
    Cancelled,
}

impl TurnStepState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StreamingModel => "streaming_model",
            Self::WaitingForClient => "waiting_for_client",
            Self::ReadyToContinue => "ready_to_continue",
            Self::Continued => "continued",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Continued | Self::Failed | Self::Cancelled)
    }

    pub fn try_from_storage(value: &str) -> Result<Self, DenError> {
        match value {
            "streaming_model" => Ok(Self::StreamingModel),
            "waiting_for_client" => Ok(Self::WaitingForClient),
            "ready_to_continue" => Ok(Self::ReadyToContinue),
            "continued" => Ok(Self::Continued),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(DenError::ValidationError(format!(
                "unsupported turn step state: {other}"
            ))),
        }
    }
}

const STEP_RETURNING: &str =
    "id, run_id, step_index, state, provider_response_id, opened_at, closed_at";
const ACTIVE_STEP_STATES_SQL: &str = "'streaming_model', 'waiting_for_client', 'ready_to_continue'";

#[derive(Debug, Clone)]
pub struct TurnStepRow {
    pub id: Uuid,
    pub run_id: String,
    pub step_index: i32,
    pub state: String,
    pub provider_response_id: Option<String>,
    pub opened_at: OffsetDateTime,
    pub closed_at: Option<OffsetDateTime>,
}

impl TurnStepRow {
    pub fn state_value(&self) -> Result<TurnStepState, DenError> {
        TurnStepState::try_from_storage(&self.state)
    }
}

fn row_to_step(row: sqlx::postgres::PgRow) -> TurnStepRow {
    TurnStepRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        step_index: row.get("step_index"),
        state: row.get("state"),
        provider_response_id: row.get("provider_response_id"),
        opened_at: row.get("opened_at"),
        closed_at: row.get("closed_at"),
    }
}

pub async fn ensure_active_step(pool: &PgPool, run_id: &str) -> Result<TurnStepRow, DenError> {
    if let Some(row) = sqlx::query(&format!(
        r"
        SELECT {STEP_RETURNING}
        FROM turn_steps
        WHERE run_id = $1
          AND state IN ({ACTIVE_STEP_STATES_SQL})
        ORDER BY step_index DESC
        LIMIT 1
        "
    ))
    .bind(run_id)
    .fetch_optional(pool)
    .await?
    {
        return Ok(row_to_step(row));
    }

    let row = sqlx::query(&format!(
        r"
        WITH next_step AS (
            SELECT COALESCE(MAX(step_index), -1) + 1 AS step_index
            FROM turn_steps
            WHERE run_id = $1
        )
        INSERT INTO turn_steps (run_id, step_index, state)
        SELECT $1, step_index, 'streaming_model'
        FROM next_step
        RETURNING {STEP_RETURNING}
        "
    ))
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(row_to_step(row))
}

pub async fn transition_step(
    pool: &PgPool,
    turn_step_id: Uuid,
    state: TurnStepState,
) -> Result<Option<TurnStepRow>, DenError> {
    let terminal = state.is_terminal();
    let row = sqlx::query(&format!(
        r"
        UPDATE turn_steps
        SET state = $2,
            closed_at = CASE WHEN $3 THEN COALESCE(closed_at, NOW()) ELSE closed_at END
        WHERE id = $1
        RETURNING {STEP_RETURNING}
        "
    ))
    .bind(turn_step_id)
    .bind(state.as_str())
    .bind(terminal)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_step))
}

pub async fn transition_active_steps_for_run(
    pool: &PgPool,
    run_id: &str,
    state: TurnStepState,
) -> Result<u64, DenError> {
    let terminal = state.is_terminal();
    let result = sqlx::query(&format!(
        r"
        UPDATE turn_steps
        SET state = $2,
            closed_at = CASE WHEN $3 THEN COALESCE(closed_at, NOW()) ELSE closed_at END
        WHERE run_id = $1
          AND state IN ({ACTIVE_STEP_STATES_SQL})
        "
    ))
    .bind(run_id)
    .bind(state.as_str())
    .bind(terminal)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
