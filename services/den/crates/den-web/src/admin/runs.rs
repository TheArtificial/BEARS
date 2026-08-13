use axum::{
    extract::{Path, Query, State},
    response::Response,
    routing::get,
    Router,
};
use den_runtime::{bearwire_events, turn_runs};
use minijinja::context;
use serde::{Deserialize, Serialize};

use crate::auth_backend::AuthSession;
use crate::errors::CustomError;
use crate::web::{self, AppState};

const RECENT_FAILED_RUN_LIMIT: i64 = 50;
const RUN_EVENT_LIMIT: i64 = 500;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/{run_id}", get(detail))
}

#[derive(Debug, Deserialize)]
struct RunLookupQuery {
    #[serde(default)]
    run_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct RunView {
    run_id: String,
    session_id: String,
    bear_id: String,
    user_id: i32,
    state: String,
    terminal_reason: Option<String>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

impl From<turn_runs::TurnRunRow> for RunView {
    fn from(row: turn_runs::TurnRunRow) -> Self {
        Self {
            run_id: row.run_id,
            session_id: row.session_id,
            bear_id: row.bear_id.to_string(),
            user_id: row.user_id,
            state: row.state,
            terminal_reason: row.terminal_reason,
            created_at: row.created_at.to_string(),
            updated_at: row.updated_at.to_string(),
            completed_at: row.completed_at.map(|value| value.to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
struct RunEventView {
    sequence_no: i64,
    event_type: String,
    session_id: String,
    created_at: String,
    event_json: String,
}

async fn index(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Query(query): Query<RunLookupQuery>,
) -> Result<Response, CustomError> {
    let failed_runs =
        turn_runs::list_recent_failed_runs(state.sqlx_pool(), RECENT_FAILED_RUN_LIMIT)
            .await?
            .into_iter()
            .map(RunView::from)
            .collect::<Vec<_>>();
    let lookup_run_id = query
        .run_id
        .as_deref()
        .map(str::trim)
        .filter(|run_id| !run_id.is_empty())
        .map(str::to_owned);
    let lookup_run = match lookup_run_id.as_deref() {
        Some(run_id) => turn_runs::get_run(state.sqlx_pool(), run_id)
            .await?
            .map(RunView::from),
        None => None,
    };

    web::render_template(
        &state,
        "admin/runs/index.html",
        auth_session,
        context! {
            failed_runs,
            lookup_run_id,
            lookup_run,
            native_runtime => true,
        },
    )
    .await
}

async fn detail(
    Path(run_id): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let run = turn_runs::get_run(state.sqlx_pool(), &run_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("turn run not found".to_string()))?;
    let events =
        bearwire_events::list_bearwire_events_for_run(state.sqlx_pool(), &run_id, RUN_EVENT_LIMIT)
            .await?
            .into_iter()
            .map(|event| RunEventView {
                sequence_no: event.sequence_no,
                event_type: event.event_type,
                session_id: event.session_id,
                created_at: event.created_at.to_string(),
                event_json: serde_json::to_string_pretty(&event.event)
                    .unwrap_or_else(|_| "{}".to_string()),
            })
            .collect::<Vec<_>>();

    web::render_template(
        &state,
        "admin/runs/detail.html",
        auth_session,
        context! {
            run => RunView::from(run),
            events,
            native_runtime => true,
        },
    )
    .await
}
