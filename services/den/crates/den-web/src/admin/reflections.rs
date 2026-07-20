use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use sqlx::Row as _;

use crate::errors::CustomError;
use crate::web::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(index))
}

pub async fn index(State(state): State<AppState>) -> Result<Response, CustomError> {
    let rows = sqlx::query(
        r"
        SELECT
            bear_slug,
            current_mode,
            count(*) AS sessions,
            count(*) FILTER (WHERE closed_at IS NULL) AS open_sessions,
            max(updated_at) AS last_updated_at
        FROM client_sessions
        GROUP BY bear_slug, current_mode
        ORDER BY bear_slug, current_mode
        ",
    )
    .fetch_all(state.sqlx_pool())
    .await?;

    let mut body = String::from(
        r#"<!doctype html><html><head><meta charset=\"utf-8\"><title>Reflection health</title></head><body><main><h1>Reflection health</h1><p><a href=\"/admin\">Admin</a></p><table><thead><tr><th>Bear</th><th>Mode</th><th>Sessions</th><th>Open</th><th>Last updated</th></tr></thead><tbody>"#,
    );

    for row in rows {
        let bear_slug: String = row.try_get("bear_slug")?;
        let current_mode: String = row.try_get("current_mode")?;
        let sessions: i64 = row.try_get("sessions")?;
        let open_sessions: i64 = row.try_get("open_sessions")?;
        let last_updated_at: Option<time::OffsetDateTime> = row.try_get("last_updated_at")?;
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(&bear_slug),
            html_escape(&current_mode),
            sessions,
            open_sessions,
            last_updated_at.map(|v| v.to_string()).unwrap_or_default()
        ));
    }

    body.push_str("</tbody></table></main></body></html>");
    Ok(Html(body).into_response())
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
