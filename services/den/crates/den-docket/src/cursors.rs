//! Per-client Docket attention cursors.
//!
//! Cursor APIs deliberately cannot mutate task or run state.

use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct DocketCursor {
    pub client_session_id: String,
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub task_id: Option<Uuid>,
    pub updated_at: OffsetDateTime,
}

pub async fn set_cursor(
    pool: &PgPool,
    client_session_id: &str,
    bear_id: Uuid,
    job_id: Uuid,
    task_id: Option<Uuid>,
) -> Result<DocketCursor, DenError> {
    if client_session_id.trim().is_empty() {
        return Err(DenError::ValidationError(
            "client session id cannot be empty".into(),
        ));
    }
    if let Some(task_id) = task_id {
        let valid: bool = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM bear_tasks WHERE id = $1 AND job_id = $2 AND bear_id = $3
            ) AS "exists!: bool"
            "#,
            task_id,
            job_id,
            bear_id
        )
        .fetch_one(pool)
        .await?;
        if !valid {
            return Err(DenError::ValidationError(
                "cursor task is not in the requested job".into(),
            ));
        }
    }
    sqlx::query_as!(
        DocketCursor,
        r#"
        INSERT INTO docket_cursors (client_session_id, bear_id, job_id, task_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (client_session_id) DO UPDATE
        SET bear_id = EXCLUDED.bear_id,
            job_id = EXCLUDED.job_id,
            task_id = EXCLUDED.task_id,
            updated_at = now()
        RETURNING
            client_session_id AS "client_session_id!: String",
            bear_id AS "bear_id!: Uuid",
            job_id AS "job_id!: Uuid",
            task_id AS "task_id?: Uuid",
            updated_at AS "updated_at!: OffsetDateTime"
        "#,
        client_session_id.trim(),
        bear_id,
        job_id,
        task_id
    )
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_cursor(
    pool: &PgPool,
    client_session_id: &str,
    bear_id: Uuid,
) -> Result<Option<DocketCursor>, DenError> {
    sqlx::query_as!(
        DocketCursor,
        r#"
        SELECT
            client_session_id AS "client_session_id!: String",
            bear_id AS "bear_id!: Uuid",
            job_id AS "job_id!: Uuid",
            task_id AS "task_id?: Uuid",
            updated_at AS "updated_at!: OffsetDateTime"
        FROM docket_cursors
        WHERE client_session_id = $1 AND bear_id = $2
        "#,
        client_session_id,
        bear_id
    )
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn clear_cursor(
    pool: &PgPool,
    client_session_id: &str,
    bear_id: Uuid,
) -> Result<bool, DenError> {
    let result = sqlx::query!(
        "DELETE FROM docket_cursors WHERE client_session_id = $1 AND bear_id = $2",
        client_session_id,
        bear_id
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}
