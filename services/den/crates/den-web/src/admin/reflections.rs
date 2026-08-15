use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};

use crate::errors::CustomError;
use crate::web::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(index))
}

pub async fn index(State(state): State<AppState>) -> Result<Response, CustomError> {
    let rows = sqlx::query!(
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
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(&row.bear_slug),
            html_escape(&row.current_mode),
            row.sessions.unwrap_or_default(),
            row.open_sessions.unwrap_or_default(),
            row.last_updated_at
                .map(|v| v.to_string())
                .unwrap_or_default()
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
