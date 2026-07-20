use axum::{extract::State, response::Response, routing::get, Router};
use minijinja::context;
use serde::Serialize;
use sqlx::Row as _;
use time::OffsetDateTime;

use crate::auth_backend::AuthSession;
use crate::errors::CustomError;
use crate::web::{self, AppState};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(index))
}

#[derive(Debug, Serialize)]
struct WorkerHealthRow {
    worker: String,
    total: i64,
    pending: i64,
    running: i64,
    succeeded: i64,
    failed: i64,
    blocked: i64,
    most_recent_at: Option<String>,
}

pub async fn index(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let mut rows = Vec::new();
    rows.push(reflection_runs_row(state.sqlx_pool()).await?);
    rows.push(compaction_events_row(state.sqlx_pool()).await?);
    rows.push(work_runs_row(state.sqlx_pool()).await?);

    web::render_template(
        &state,
        "admin/workers/index.html",
        auth_session,
        context! {
            rows,
            native_runtime => true,
        },
    )
    .await
}

async fn reflection_runs_row(pool: &sqlx::PgPool) -> Result<WorkerHealthRow, CustomError> {
    let row = sqlx::query(
        r"
        SELECT
            count(*) AS total,
            0::BIGINT AS pending,
            count(*) FILTER (WHERE status = 'started') AS running,
            count(*) FILTER (WHERE status = 'completed') AS succeeded,
            count(*) FILTER (WHERE status = 'failed') AS failed,
            count(*) FILTER (WHERE status = 'skipped') AS blocked,
            max(coalesce(completed_at, created_at)) AS most_recent_at
        FROM pair_reflection_runs
        ",
    )
    .fetch_one(pool)
    .await?;

    worker_health_row("Reflection runs", row)
}

async fn compaction_events_row(pool: &sqlx::PgPool) -> Result<WorkerHealthRow, CustomError> {
    let row = sqlx::query(
        r"
        SELECT
            count(*) AS total,
            0::BIGINT AS pending,
            0::BIGINT AS running,
            count(*) FILTER (WHERE status = 'Applied') AS succeeded,
            count(*) FILTER (WHERE status = 'Failed') AS failed,
            count(*) FILTER (WHERE status = 'Skipped') AS blocked,
            max(created_at) AS most_recent_at
        FROM runtime_compaction_events
        ",
    )
    .fetch_one(pool)
    .await?;

    worker_health_row("Compaction events", row)
}

async fn work_runs_row(pool: &sqlx::PgPool) -> Result<WorkerHealthRow, CustomError> {
    let row = sqlx::query(
        r"
        SELECT
            count(*) AS total,
            count(*) FILTER (WHERE state = 'queued') AS pending,
            count(*) FILTER (WHERE state IN ('claimed', 'provisioning', 'running', 'reporting')) AS running,
            count(*) FILTER (WHERE state = 'succeeded') AS succeeded,
            count(*) FILTER (WHERE state IN ('failed', 'timed_out')) AS failed,
            count(*) FILTER (WHERE state IN ('blocked', 'cancelled')) AS blocked,
            max(updated_at) AS most_recent_at
        FROM bear_work_runs
        ",
    )
    .fetch_one(pool)
    .await?;

    worker_health_row("Work runs", row)
}

fn worker_health_row(
    worker: &str,
    row: sqlx::postgres::PgRow,
) -> Result<WorkerHealthRow, CustomError> {
    let most_recent_at: Option<OffsetDateTime> = row.try_get("most_recent_at")?;
    Ok(WorkerHealthRow {
        worker: worker.to_string(),
        total: row.try_get("total")?,
        pending: row.try_get("pending")?,
        running: row.try_get("running")?,
        succeeded: row.try_get("succeeded")?,
        failed: row.try_get("failed")?,
        blocked: row.try_get("blocked")?,
        most_recent_at: most_recent_at.map(|value| value.to_string()),
    })
}
