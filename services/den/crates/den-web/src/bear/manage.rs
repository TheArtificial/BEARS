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
use den_core::{
    client_tools::ClientToolName,
    tools::descriptor::builtin_den_tool_descriptors,
    BearProfile,
};
use minijinja::context;
use serde::Serialize;

use crate::{
    auth_backend::AuthSession,
    errors::CustomError,
    web::{self, AppState},
};

use super::settings::{bear_nav_context, load_session_bear};

#[derive(Serialize)]
struct ToolStanceRow {
    name: String,
    tools: Vec<ToolRow>,
}

#[derive(Serialize)]
struct ToolRow {
    name: &'static str,
    origin: &'static str,
    note: &'static str,
}

fn stance_label(stance: BearProfile) -> &'static str {
    match stance {
        BearProfile::Chat => "Chat",
        BearProfile::Pair => "Pair",
        BearProfile::Curate => "Curate",
        BearProfile::Work => "Work",
        BearProfile::Watch => "Watch",
    }
}

/// Build the per-stance tool roster shown on the Tools management page.
///
/// Source: `den_core::tools::descriptor::builtin_den_tool_descriptors()` for
/// Den-hosted tools (each descriptor carries `allowed_roles`, so filtering
/// per stance is exact), plus `den_core::client_tools::ClientToolName::all()`
/// for armature-local tools, which are only ever exposed on the pair stance.
fn tool_stances_context() -> Vec<ToolStanceRow> {
    let builtin = builtin_den_tool_descriptors();

    BearProfile::ALL
        .iter()
        .map(|stance| {
            let mut tools: Vec<ToolRow> = builtin
                .iter()
                .filter(|descriptor| descriptor.allowed_roles.contains(&stance.as_str()))
                .map(|descriptor| ToolRow {
                    name: descriptor.name,
                    origin: "built-in",
                    note: descriptor.label,
                })
                .collect();
            if *stance == BearProfile::Pair {
                tools.extend(ClientToolName::all().iter().map(|tool| {
                    let descriptor = tool.descriptor();
                    ToolRow {
                        name: descriptor.provider_name,
                        origin: "local (armature)",
                        note: descriptor.title,
                    }
                }));
            }
            ToolStanceRow {
                name: stance_label(*stance).to_string(),
                tools,
            }
        })
        .collect()
}

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
    let stances: Vec<&'static str> = BearProfile::ALL.iter().map(|p| p.as_str()).collect();
    web::render_template(
        &state,
        "bear/manage/identity.html",
        auth_session,
        context! {
            can_manage_bear,
            stances,
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
    let tool_stances = tool_stances_context();
    web::render_template(
        &state,
        "bear/manage/tools.html",
        auth_session,
        context! {
            can_manage_bear,
            manage_title => "Tools",
            tool_stances,
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
