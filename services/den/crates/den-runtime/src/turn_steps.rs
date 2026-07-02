use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

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

pub async fn ensure_active_step(
    pool: &PgPool,
    run_id: &str,
) -> Result<TurnStepRow, DenError> {
    if let Some(row) = sqlx::query(
        r#"
        SELECT id, run_id, step_index, state, provider_response_id, opened_at, closed_at
        FROM turn_steps
        WHERE run_id = $1
          AND state IN ('streaming_model', 'waiting_for_client', 'ready_to_continue')
        ORDER BY step_index DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?
    {
        return Ok(row_to_step(row));
    }

    let row = sqlx::query(
        r#"
        WITH next_step AS (
            SELECT COALESCE(MAX(step_index), -1) + 1 AS step_index
            FROM turn_steps
            WHERE run_id = $1
        )
        INSERT INTO turn_steps (run_id, step_index, state)
        SELECT $1, step_index, 'streaming_model'
        FROM next_step
        RETURNING id, run_id, step_index, state, provider_response_id, opened_at, closed_at
        "#,
    )
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(row_to_step(row))
}

pub async fn transition_step(
    pool: &PgPool,
    turn_step_id: Uuid,
    state: &str,
) -> Result<Option<TurnStepRow>, DenError> {
    let terminal = matches!(state, "continued" | "failed" | "cancelled");
    let row = sqlx::query(
        r#"
        UPDATE turn_steps
        SET state = $2,
            closed_at = CASE WHEN $3 THEN COALESCE(closed_at, NOW()) ELSE closed_at END
        WHERE id = $1
        RETURNING id, run_id, step_index, state, provider_response_id, opened_at, closed_at
        "#,
    )
    .bind(turn_step_id)
    .bind(state)
    .bind(terminal)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_step))
}

pub async fn transition_active_steps_for_run(
    pool: &PgPool,
    run_id: &str,
    state: &str,
) -> Result<u64, DenError> {
    let terminal = matches!(state, "continued" | "failed" | "cancelled");
    let result = sqlx::query(
        r#"
        UPDATE turn_steps
        SET state = $2,
            closed_at = CASE WHEN $3 THEN COALESCE(closed_at, NOW()) ELSE closed_at END
        WHERE run_id = $1
          AND state IN ('streaming_model', 'waiting_for_client', 'ready_to_continue')
        "#,
    )
    .bind(run_id)
    .bind(state)
    .bind(terminal)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
