// ROUTES: When modifying routes in this file, update /src/ROUTES.md.
//! Cabinet wiki UI: Den's shared, human-editable knowledge layer (Phase 1).
//!
//! Every logged-in user can browse, create, and edit items; each edit
//! publishes an immutable version through the same `den_service::cabinet`
//! facade the model tools use. Stale-base edits re-render the edit form with
//! the conflict, never merging silently.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use axum_extra::extract::Form;
use minijinja::context;
use serde::Deserialize;

use crate::{
    auth_backend::AuthSession,
    errors::CustomError,
    web::{self, AppState},
};
use den_cabinet::{
    ActorScope, CabinetError, CabinetItemRef, CabinetVersionRef, CreateItemRequest,
    HistoryRequest, ItemKind, Lifecycle, ReadRequest, SearchFilters, SearchRequest,
    UpdateItemRequest,
};
use den_core::ids::UserId;
use den_service::cabinet as cabinet_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/cabinet", get(index))
        .route("/cabinet/new", get(new_form).post(create))
        .route("/cabinet/{cabinet_ref}", get(item))
        .route("/cabinet/{cabinet_ref}/edit", get(edit_form).post(update))
        .route("/cabinet/{cabinet_ref}/history", get(history))
        .route("/cabinet/{cabinet_ref}/archive", axum::routing::post(archive))
        .route("/cabinet/{cabinet_ref}/restore", axum::routing::post(restore))
}

fn require_user_scope(auth_session: &AuthSession) -> Result<ActorScope, CustomError> {
    auth_session
        .user
        .as_ref()
        .map(|user| ActorScope::user(UserId(user.id)))
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))
}

fn cabinet_error(error: CabinetError) -> CustomError {
    CustomError::from(den_core::DenError::from(error))
}

fn parse_item_ref(value: &str) -> Result<CabinetItemRef, CustomError> {
    CabinetItemRef::parse(value).map_err(|_| CustomError::NotFound("no such item".to_string()))
}

fn item_url(cabinet_ref: &CabinetItemRef) -> String {
    format!("/cabinet/{}", cabinet_ref.as_str())
}

#[derive(Debug, Default, Deserialize)]
struct IndexQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    lifecycle: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

async fn index(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Query(query): Query<IndexQuery>,
) -> Result<Response, CustomError> {
    let scope = require_user_scope(&auth_session)?;
    let archived = query.lifecycle.as_deref() == Some("archived");
    let items = cabinet_service::search(
        state.sqlx_pool(),
        SearchRequest {
            scope,
            query: query.q.clone().unwrap_or_default(),
            filters: SearchFilters {
                lifecycle: Some(if archived {
                    Lifecycle::Archived
                } else {
                    Lifecycle::Active
                }),
                ..SearchFilters::default()
            },
        },
    )
    .await
    .map_err(cabinet_error)?;
    let items: Vec<serde_json::Value> = items
        .into_iter()
        .map(|item| {
            serde_json::json!({
                "cabinet_ref": item.cabinet_ref.as_str(),
                "title": item.title,
                "updated_at": item.updated_at,
            })
        })
        .collect();
    web::render_template(
        &state,
        "cabinet/index.html",
        auth_session,
        context! {
            title => "Cabinet",
            items => items,
            q => query.q,
            archived => archived,
            message => query.message,
        },
    )
    .await
}

async fn new_form(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    require_user_scope(&auth_session)?;
    web::render_template(
        &state,
        "cabinet/new.html",
        auth_session,
        context! { title => "New Cabinet item" },
    )
    .await
}

#[derive(Debug, Deserialize)]
struct CreateForm {
    title: String,
    content: String,
}

async fn create(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<CreateForm>,
) -> Result<Response, CustomError> {
    let scope = require_user_scope(&auth_session)?;
    let view = cabinet_service::create_item(
        state.sqlx_pool(),
        CreateItemRequest {
            scope,
            kind: ItemKind::Document,
            title: form.title,
            content: form.content,
            collection_ref: None,
            mission_ref: None,
            source_links: Vec::new(),
        },
    )
    .await
    .map_err(cabinet_error)?;
    Ok(Redirect::to(&item_url(&view.item.cabinet_ref)).into_response())
}

#[derive(Debug, Default, Deserialize)]
struct ItemQuery {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

async fn item(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(cabinet_ref): Path<String>,
    Query(query): Query<ItemQuery>,
) -> Result<Response, CustomError> {
    let scope = require_user_scope(&auth_session)?;
    let cabinet_ref = parse_item_ref(&cabinet_ref)?;
    let version_ref = query
        .version
        .as_deref()
        .map(CabinetVersionRef::parse)
        .transpose()
        .map_err(|_| CustomError::NotFound("no such version".to_string()))?;
    let view = cabinet_service::read(
        state.sqlx_pool(),
        ReadRequest {
            scope,
            cabinet_ref: cabinet_ref.clone(),
            version_ref: version_ref.clone(),
        },
    )
    .await
    .map_err(cabinet_error)?;
    let is_current = view.item.current_version.as_ref() == Some(view.version.version_ref());
    let sources: Vec<serde_json::Value> = view
        .sources
        .iter()
        .map(|source| {
            serde_json::json!({
                "kind": source.source_kind,
                "locator": source.locator,
                "role": source.role,
            })
        })
        .collect();
    web::render_template(
        &state,
        "cabinet/item.html",
        auth_session,
        context! {
            title => view.item.title,
            cabinet_ref => cabinet_ref.as_str(),
            item_title => view.item.title,
            content => view.version.content(),
            revision => view.version.revision(),
            version_ref => view.version.version_ref().as_str(),
            is_current => is_current,
            lifecycle => view.item.lifecycle,
            authored_by => view.version.authored_by(),
            authored_at => view.version.authored_at(),
            sources => sources,
            message => query.message,
        },
    )
    .await
}

#[derive(Debug, Default, Deserialize)]
struct EditQuery {
    #[serde(default)]
    error: Option<String>,
}

async fn edit_form(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(cabinet_ref): Path<String>,
    Query(query): Query<EditQuery>,
) -> Result<Response, CustomError> {
    let scope = require_user_scope(&auth_session)?;
    let cabinet_ref = parse_item_ref(&cabinet_ref)?;
    let view = cabinet_service::read(
        state.sqlx_pool(),
        ReadRequest {
            scope,
            cabinet_ref,
            version_ref: None,
        },
    )
    .await
    .map_err(cabinet_error)?;
    render_edit_form(
        &state,
        auth_session,
        &view.item.cabinet_ref,
        &view.item.title,
        view.version.content(),
        view.version.version_ref().as_str(),
        query.error.as_deref(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn render_edit_form(
    state: &AppState,
    auth_session: AuthSession,
    cabinet_ref: &CabinetItemRef,
    title: &str,
    content: &str,
    base_version: &str,
    error: Option<&str>,
) -> Result<Response, CustomError> {
    web::render_template(
        state,
        "cabinet/edit.html",
        auth_session,
        context! {
            title => format!("Edit: {title}"),
            cabinet_ref => cabinet_ref.as_str(),
            item_title => title,
            content => content,
            base_version => base_version,
            error => error,
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
struct EditForm {
    title: String,
    content: String,
    base_version: String,
}

async fn update(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(cabinet_ref): Path<String>,
    Form(form): Form<EditForm>,
) -> Result<Response, CustomError> {
    let scope = require_user_scope(&auth_session)?;
    let cabinet_ref = parse_item_ref(&cabinet_ref)?;
    let base_version = CabinetVersionRef::parse(&form.base_version)
        .map_err(|error| CustomError::ValidationError(error.to_string()))?;
    let result = cabinet_service::update_item(
        state.sqlx_pool(),
        UpdateItemRequest {
            scope,
            cabinet_ref: cabinet_ref.clone(),
            content: form.content.clone(),
            base_version,
            title: Some(form.title.clone()),
        },
    )
    .await;
    match result {
        Ok(view) => Ok(Redirect::to(&format!(
            "{}?message={}",
            item_url(&view.item.cabinet_ref),
            urlencoding::encode("Published a new revision.")
        ))
        .into_response()),
        Err(CabinetError::Conflict { current_version }) => {
            // Someone published first: hand the editor back their draft with
            // the new base so they can reconcile explicitly.
            render_edit_form(
                &state,
                auth_session,
                &cabinet_ref,
                &form.title,
                &form.content,
                current_version.as_str(),
                Some(
                    "Someone else published a newer revision while you were editing. \
                     Your draft is preserved below; review the latest revision before saving.",
                ),
            )
            .await
        }
        Err(error) => Err(cabinet_error(error)),
    }
}

async fn history(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(cabinet_ref): Path<String>,
) -> Result<Response, CustomError> {
    let scope = require_user_scope(&auth_session)?;
    let cabinet_ref = parse_item_ref(&cabinet_ref)?;
    let item_view = cabinet_service::read(
        state.sqlx_pool(),
        ReadRequest {
            scope: scope.clone(),
            cabinet_ref: cabinet_ref.clone(),
            version_ref: None,
        },
    )
    .await
    .map_err(cabinet_error)?;
    let versions = cabinet_service::history(
        state.sqlx_pool(),
        HistoryRequest {
            scope,
            cabinet_ref: cabinet_ref.clone(),
        },
    )
    .await
    .map_err(cabinet_error)?;
    let versions: Vec<serde_json::Value> = versions
        .into_iter()
        .map(|version| {
            serde_json::json!({
                "version_ref": version.version_ref.as_str(),
                "revision": version.revision,
                "authored_by": version.authored_by,
                "authored_at": version.authored_at,
            })
        })
        .collect();
    web::render_template(
        &state,
        "cabinet/history.html",
        auth_session,
        context! {
            title => format!("History: {}", item_view.item.title),
            cabinet_ref => cabinet_ref.as_str(),
            item_title => item_view.item.title,
            versions => versions,
        },
    )
    .await
}

async fn archive(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(cabinet_ref): Path<String>,
) -> Result<Response, CustomError> {
    let scope = require_user_scope(&auth_session)?;
    let cabinet_ref = parse_item_ref(&cabinet_ref)?;
    cabinet_service::archive_item(state.sqlx_pool(), &scope, &cabinet_ref)
        .await
        .map_err(cabinet_error)?;
    Ok(Redirect::to(&format!(
        "/cabinet?message={}",
        urlencoding::encode("Item archived.")
    ))
    .into_response())
}

async fn restore(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(cabinet_ref): Path<String>,
) -> Result<Response, CustomError> {
    let scope = require_user_scope(&auth_session)?;
    let cabinet_ref = parse_item_ref(&cabinet_ref)?;
    cabinet_service::restore_item(state.sqlx_pool(), &scope, &cabinet_ref)
        .await
        .map_err(cabinet_error)?;
    Ok(Redirect::to(&format!(
        "{}?message={}",
        item_url(&cabinet_ref),
        urlencoding::encode("Item restored.")
    ))
    .into_response())
}
