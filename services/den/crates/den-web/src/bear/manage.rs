//! New-IA bear management areas: identity, skills, tools, connections,
//! portability — plus redirects from retired paths. Member-gated like the
//! rest of `/bear/{slug}/…` (read for members, write for bear admins).
//!
//! When changing routes, update `src/web/ROUTES.md`.

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use axum_extra::routing::RouterExt;
use minijinja::context;

use crate::{
    auth_backend::AuthSession,
    errors::CustomError,
    web::{self, AppState},
};

use super::settings::{bear_nav_context, load_session_bear};

pub fn router() -> Router<AppState> {
    Router::new()
        .route_with_tsr("/bear/{slug}/identity", get(identity_view))
        .route_with_tsr("/bear/{slug}/skills", get(skills_view))
        .route_with_tsr("/bear/{slug}/tools", get(tools_view))
        .route_with_tsr("/bear/{slug}/connections", get(connections_view))
        .route_with_tsr("/bear/{slug}/portability", get(portability_view))
        // Retired paths from the previous IA.
        .route_with_tsr("/bear/{slug}/access", get(redirect_people))
        .route_with_tsr("/bear/{slug}/policy", get(redirect_resources))
}

async fn redirect_people(Path(slug): Path<String>) -> Redirect {
    Redirect::permanent(&format!("/bear/{slug}/people"))
}

async fn redirect_resources(Path(slug): Path<String>) -> Redirect {
    Redirect::permanent(&format!("/bear/{slug}/resources"))
}

async fn identity_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) =
        match super::settings::load_session_bear(&state, &auth_session, &slug).await? {
            Ok(v) => v,
            Err(r) => return Ok(r.into_response()),
        };
    web::render_template(
        &state,
        "bear/manage/identity.html",
        auth_session,
        context! {
            can_manage_bear,
            manage_title => "Identity & charter",
            ..bear_nav_context(&bear, "identity"),
        },
    )
    .await
}

async fn skills_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    web::render_template(
        &state,
        "bear/manage/skills.html",
        auth_session,
        context! {
            can_manage_bear,
            manage_title => "Skills",
            ..bear_nav_context(&bear, "skills"),
        },
    )
    .await
}

async fn tools_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    web::render_template(
        &state,
        "bear/manage/tools.html",
        auth_session,
        context! {
            can_manage_bear,
            manage_title => "Tools",
            ..bear_nav_context(&bear, "tools"),
        },
    )
    .await
}

async fn connections_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    web::render_template(
        &state,
        "bear/manage/connections.html",
        auth_session,
        context! {
            can_manage_bear,
            manage_title => "Connections",
            ..bear_nav_context(&bear, "connections"),
        },
    )
    .await
}

async fn portability_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    web::render_template(
        &state,
        "bear/manage/portability.html",
        auth_session,
        context! {
            can_manage_bear,
            manage_title => "Portability",
            ..bear_nav_context(&bear, "portability"),
        },
    )
    .await
}
