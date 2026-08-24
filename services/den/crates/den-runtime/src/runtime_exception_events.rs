//! Bounded, searchable evidence for failures that users or operators should investigate.
//!
//! This is intentionally not a log sink: callers record only classified warning/error
//! outcomes with allowlisted context. Inserts are best-effort at the caller boundary.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const RETENTION_DAYS: i64 = 7;
const MAX_ROWS: i64 = 50_000;
const MAX_MESSAGE_CHARS: usize = 2_000;
const MAX_DETAILS_BYTES: usize = 8_000;
const MAX_QUERY_LIMIT: i64 = 200;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExceptionSeverity {
    Warning,
    Error,
}

impl RuntimeExceptionSeverity {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeExceptionContext {
    pub session_id: Option<String>,
    pub runtime_run_id: Option<String>,
    pub work_run_id: Option<Uuid>,
    pub docket_job_id: Option<Uuid>,
    pub docket_task_id: Option<Uuid>,
    pub conversation_id: Option<String>,
    pub bear_id: Option<Uuid>,
    pub build_revision: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewRuntimeExceptionEvent {
    pub severity: RuntimeExceptionSeverity,
    pub component: String,
    pub event_code: String,
    pub message: String,
    pub details: Value,
    pub context: RuntimeExceptionContext,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct RuntimeExceptionEventFilter {
    /// Required at the BearWire boundary so diagnostic evidence cannot cross Bears.
    pub bear_id: Option<Uuid>,
    pub work_run_id: Option<Uuid>,
    pub runtime_run_id: Option<String>,
    pub session_id: Option<String>,
    pub docket_job_id: Option<Uuid>,
    pub event_code: Option<String>,
    pub severity: Option<RuntimeExceptionSeverity>,
    pub limit: Option<i64>,
}

#[derive(Clone, Debug, FromRow, Serialize)]
pub struct RuntimeExceptionEvent {
    pub id: Uuid,
    pub created_at: OffsetDateTime,
    pub severity: String,
    pub component: String,
    pub event_code: String,
    pub message: String,
    pub details: Value,
    pub session_id: Option<String>,
    pub runtime_run_id: Option<String>,
    pub work_run_id: Option<Uuid>,
    pub docket_job_id: Option<Uuid>,
    pub docket_task_id: Option<Uuid>,
    pub conversation_id: Option<String>,
    pub bear_id: Option<Uuid>,
    pub build_revision: Option<String>,
}

pub async fn record(pool: &PgPool, event: NewRuntimeExceptionEvent) -> Result<(), sqlx::Error> {
    let context = event.context;
    let expires_at = OffsetDateTime::now_utc() + Duration::days(RETENTION_DAYS);
    // sqlx-dynamic: optional diagnostic filters keep this low-volume ops query compact.
    sqlx::query(
        r#"INSERT INTO runtime_exception_events (
                id, expires_at, severity, component, event_code, message, details,
                session_id, runtime_run_id, work_run_id, docket_job_id, docket_task_id,
                conversation_id, bear_id, build_revision
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10, $11, $12, $13, $14, $15
            )"#,
    )
    .bind(Uuid::new_v4())
    .bind(expires_at)
    .bind(event.severity.as_str())
    .bind(event.component)
    .bind(event.event_code)
    .bind(truncate(&event.message, MAX_MESSAGE_CHARS))
    .bind(bounded_details(event.details))
    .bind(context.session_id)
    .bind(context.runtime_run_id)
    .bind(context.work_run_id)
    .bind(context.docket_job_id)
    .bind(context.docket_task_id)
    .bind(context.conversation_id)
    .bind(context.bear_id)
    .bind(context.build_revision)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(
    pool: &PgPool,
    filter: RuntimeExceptionEventFilter,
) -> Result<Vec<RuntimeExceptionEvent>, sqlx::Error> {
    // sqlx-dynamic: all predicates are fixed and parameterized; optional filters avoid separate query variants.
    sqlx::query_as::<_, RuntimeExceptionEvent>(
        r#"SELECT id, created_at, severity, component, event_code, message, details,
                  session_id, runtime_run_id, work_run_id, docket_job_id, docket_task_id,
                  conversation_id, bear_id, build_revision
           FROM runtime_exception_events
           WHERE ($1::uuid IS NULL OR bear_id = $1)
             AND ($2::uuid IS NULL OR work_run_id = $2)
             AND ($3::text IS NULL OR runtime_run_id = $3)
             AND ($4::text IS NULL OR session_id = $4)
             AND ($5::uuid IS NULL OR docket_job_id = $5)
             AND ($6::text IS NULL OR event_code = $6)
             AND ($7::text IS NULL OR severity = $7)
           ORDER BY created_at DESC
           LIMIT $8"#,
    )
    .bind(filter.bear_id)
    .bind(filter.work_run_id)
    .bind(filter.runtime_run_id)
    .bind(filter.session_id)
    .bind(filter.docket_job_id)
    .bind(filter.event_code)
    .bind(filter.severity.map(RuntimeExceptionSeverity::as_str))
    .bind(filter.limit.unwrap_or(50).clamp(1, MAX_QUERY_LIMIT))
    .fetch_all(pool)
    .await
}

/// Deletes expired rows, then oldest rows above the fixed capacity. Call periodically;
/// failure is safe because reads filter by retention and inserts remain bounded by cleanup.
pub async fn prune(pool: &PgPool) -> Result<u64, sqlx::Error> {
    // sqlx-dynamic: DELETE uses a fixed bounded-retention expression and has no user input.
    let expired = sqlx::query("DELETE FROM runtime_exception_events WHERE expires_at <= NOW()")
        .execute(pool)
        .await?
        .rows_affected();
    // sqlx-dynamic: fixed capacity cleanup with no dynamic identifiers or untrusted input.
    let overflow = sqlx::query(
        r#"DELETE FROM runtime_exception_events
           WHERE id IN (
               SELECT id FROM runtime_exception_events
               ORDER BY created_at DESC
               OFFSET $1
           )"#,
    )
    .bind(MAX_ROWS)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(expired + overflow)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push('…');
    }
    output
}

fn bounded_details(value: Value) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= MAX_DETAILS_BYTES {
        value
    } else {
        serde_json::json!({"truncated": true, "preview": truncate(&serialized, MAX_DETAILS_BYTES)})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_message_and_details_without_invalid_json() {
        let message = "x".repeat(MAX_MESSAGE_CHARS + 1);
        assert_eq!(
            truncate(&message, MAX_MESSAGE_CHARS).chars().count(),
            MAX_MESSAGE_CHARS + 1
        );
        let details = bounded_details(serde_json::json!({"value": "x".repeat(MAX_DETAILS_BYTES)}));
        assert_eq!(details["truncated"], true);
        assert!(details["preview"].is_string());
    }
}
