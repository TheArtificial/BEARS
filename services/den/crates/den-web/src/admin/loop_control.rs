use axum::{extract::State, response::Response, routing::get, Router};
use den_runtime::summarize_recent_loop_control_replay_profile;
use minijinja::context;
use serde::Serialize;
use time::{Duration, OffsetDateTime};

use crate::auth_backend::AuthSession;
use crate::errors::CustomError;
use crate::web::{self, AppState};

const SUMMARY_WINDOW_DAYS: i64 = 30;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(index))
}

#[derive(Debug, Serialize)]
struct CountRow {
    value: String,
    count: usize,
}

pub async fn index(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let now = OffsetDateTime::now_utc();
    let since = now - Duration::days(SUMMARY_WINDOW_DAYS);
    let summary = summarize_recent_loop_control_replay_profile(state.sqlx_pool(), since).await?;

    web::render_template(
        &state,
        "admin/loop_control/index.html",
        auth_session,
        context! {
            since => since.to_string(),
            until => now.to_string(),
            turn_count => summary.turn_count,
            decision_count => summary.decision_count,
            decision_kinds => summary.decision_kind_counts.into_iter().map(|row| CountRow {
                value: row.value.as_str().to_string(),
                count: row.count,
            }).collect::<Vec<_>>(),
            control_levels => summary.control_level_counts.into_iter().map(|row| CountRow {
                value: row.value,
                count: row.count,
            }).collect::<Vec<_>>(),
            orientations => summary.orientation_kind_counts.into_iter().map(|row| CountRow {
                value: row.value,
                count: row.count,
            }).collect::<Vec<_>>(),
            reasons => summary.reason_counts.into_iter().map(|row| CountRow {
                value: row.value,
                count: row.count,
            }).collect::<Vec<_>>(),
            native_runtime => true,
        },
    )
    .await
}
