// ROUTES: When modifying routes in this file, update /src/web/ROUTES.md if present.
pub mod api;
pub mod bears;
pub mod membership;
pub mod models;
pub mod oauth_clients;
pub mod reflections;
pub mod sandbox_images;
pub mod users;
pub mod workers;

use axum::response::Response;
use axum::{extract::State, routing::get, Router};
use minijinja::context;

use crate::errors::CustomError;
use crate::web::{self, AppState};
use crate::{auth_backend::AuthSession, core::user};

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(users::router())
        .merge(oauth_clients::router())
        .merge(bears::router())
        .merge(membership::router())
        .merge(models::router())
        .merge(sandbox_images::router())
        .nest("/workers", workers::router())
        .nest("/api", api::router())
        .route("/", get(admin_home))
}

async fn admin_home(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let users = user::db::get_users(&state.sqlx_pool).await?;

    web::render_template(
        &state,
        "admin/menu.html",
        auth_session,
        context! {
            users => users,
            native_runtime => true,
        },
    )
    .await
}
