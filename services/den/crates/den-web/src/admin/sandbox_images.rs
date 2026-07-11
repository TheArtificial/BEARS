// ROUTES: When modifying routes in this file, update /src/ROUTES.md.
//! Site-admin sandbox image-store management: what the dind engine holds,
//! registry pulls, one-click builds of the shipped image variants, and the
//! Den-managed image catalog (the dispatch trust boundary — jobs select
//! images by catalog name only).

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use axum_extra::extract::Form;
use minijinja::context;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth_backend::AuthSession,
    errors::CustomError,
    web::{self, AppState},
    work::surfaces::push_surfaces_best_effort,
};
use den_sandbox::protocol::{BuildImageRequest, BuildVariant};
use den_sandbox::SandboxClient;
use den_service::work_surfaces;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/sandbox", get(index))
        .route("/sandbox/pull", post(pull))
        .route("/sandbox/build", post(build))
        .route("/sandbox/images/remove", post(remove_image))
        .route("/sandbox/operations/{operation_id}", get(operation))
        .route("/sandbox/catalog", post(catalog_create))
        .route("/sandbox/catalog/{image_id}/update", post(catalog_update))
        .route("/sandbox/catalog/{image_id}/delete", post(catalog_delete))
        .route("/sandbox/catalog/{image_id}/default", post(catalog_default))
}

fn provider_client(state: &AppState) -> Option<SandboxClient> {
    let url = state
        .config
        .sandbox_server_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())?;
    Some(SandboxClient::new(url, &state.config.sandbox_server_token))
}

fn redirect_with(message: &str) -> Response {
    Redirect::to(&format!(
        "/admin/sandbox?message={}",
        urlencoding::encode(message)
    ))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct MessageQuery {
    #[serde(default)]
    message: Option<String>,
}

async fn index(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Query(query): Query<MessageQuery>,
) -> Result<Response, CustomError> {
    let catalog = work_surfaces::list_catalog_images(state.sqlx_pool()).await?;

    // Provider-side data is best-effort: the catalog stays editable when the
    // provider is down.
    let client = provider_client(&state);
    let (provider_error, store, operations) = match &client {
        None => (
            Some("no sandbox provider configured (SANDBOX_SERVER_URL unset)".to_string()),
            None,
            Vec::new(),
        ),
        Some(client) => match client.list_images().await {
            Ok(store) => {
                let operations = client.list_operations().await.unwrap_or_default();
                (None, Some(store), operations)
            }
            Err(err) => (Some(err.to_string()), None, Vec::new()),
        },
    };

    // Which catalog references exist in the engine store right now.
    let engine_refs: std::collections::BTreeSet<String> = store
        .as_ref()
        .map(|store| {
            store
                .images
                .iter()
                .flat_map(|image| {
                    [
                        format!("{}:{}", image.repository, image.tag),
                        image.repository.clone(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    let catalog: Vec<serde_json::Value> = catalog
        .into_iter()
        .map(|image| {
            serde_json::json!({
                "id": image.id.to_string(),
                "name": image.name,
                "image_ref": image.image_ref,
                "description": image.description,
                "is_default": image.is_default,
                "in_engine": store.is_some() && engine_refs.contains(&image.image_ref),
                "engine_known": store.is_some(),
            })
        })
        .collect();

    web::render_template(
        &state,
        "admin/sandbox/index.html",
        auth_session,
        context! {
            title => "Sandbox images",
            provider_error => provider_error,
            provider_url => state.config.sandbox_server_url,
            store => store,
            operations => operations,
            catalog => catalog,
            message => query.message,
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
struct PullForm {
    reference: String,
}

async fn pull(
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Form(form): Form<PullForm>,
) -> Result<Response, CustomError> {
    let Some(client) = provider_client(&state) else {
        return Ok(redirect_with("No sandbox provider configured."));
    };
    let accepted = client
        .pull_image(form.reference.trim())
        .await
        .map_err(|err| CustomError::ValidationError(format!("pull rejected: {err}")))?;
    Ok(Redirect::to(&format!(
        "/admin/sandbox/operations/{}",
        accepted.operation_id
    ))
    .into_response())
}

#[derive(Debug, Deserialize)]
struct BuildForm {
    variant: String,
}

async fn build(
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Form(form): Form<BuildForm>,
) -> Result<Response, CustomError> {
    let Some(client) = provider_client(&state) else {
        return Ok(redirect_with("No sandbox provider configured."));
    };
    let variant = match form.variant.as_str() {
        "base" => BuildVariant::Base,
        "rust" => BuildVariant::Rust,
        "node" => BuildVariant::Node,
        "godot" => BuildVariant::Godot,
        other => {
            return Err(CustomError::ValidationError(format!(
                "unknown build variant '{other}'"
            )))
        }
    };
    let accepted = client
        .build_image(&BuildImageRequest { variant })
        .await
        .map_err(|err| CustomError::ValidationError(format!("build rejected: {err}")))?;
    Ok(Redirect::to(&format!(
        "/admin/sandbox/operations/{}",
        accepted.operation_id
    ))
    .into_response())
}

#[derive(Debug, Deserialize)]
struct RemoveForm {
    reference: String,
}

async fn remove_image(
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Form(form): Form<RemoveForm>,
) -> Result<Response, CustomError> {
    let Some(client) = provider_client(&state) else {
        return Ok(redirect_with("No sandbox provider configured."));
    };
    let message = match client.remove_image(form.reference.trim()).await {
        Ok(()) => format!("Removed {} from the engine store.", form.reference.trim()),
        Err(err) => format!("Remove failed: {err}"),
    };
    Ok(redirect_with(&message))
}

async fn operation(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(operation_id): Path<String>,
) -> Result<Response, CustomError> {
    let Some(client) = provider_client(&state) else {
        return Ok(redirect_with("No sandbox provider configured."));
    };
    let (operation, unknown) = match client.get_operation(&operation_id).await {
        Ok(descriptor) => (Some(descriptor), false),
        Err(err) if err.kind() == Some("unknown_operation") => (None, true),
        Err(err) => return Err(CustomError::System(format!("operation fetch failed: {err}"))),
    };
    let running = operation
        .as_ref()
        .is_some_and(|op| matches!(op.state, den_sandbox::protocol::OperationState::Running));
    web::render_template(
        &state,
        "admin/sandbox/operation.html",
        auth_session,
        context! {
            title => "Sandbox operation",
            operation => operation,
            unknown => unknown,
            running => running,
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
struct CatalogCreateForm {
    name: String,
    image_ref: String,
    #[serde(default)]
    description: String,
}

async fn catalog_create(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<CatalogCreateForm>,
) -> Result<Response, CustomError> {
    let user_id = auth_session
        .user
        .as_ref()
        .map(|user| user.id)
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))?;
    let description = form.description.trim();
    work_surfaces::create_catalog_image(
        state.sqlx_pool(),
        form.name.trim(),
        form.image_ref.trim(),
        if description.is_empty() {
            None
        } else {
            Some(description)
        },
        user_id,
    )
    .await?;
    let note = push_surfaces_best_effort(&state).await.unwrap_or_default();
    Ok(redirect_with(&format!("Catalog entry added.{note}")))
}

#[derive(Debug, Deserialize)]
struct CatalogUpdateForm {
    image_ref: String,
    #[serde(default)]
    description: String,
}

async fn catalog_update(
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Path(image_id): Path<Uuid>,
    Form(form): Form<CatalogUpdateForm>,
) -> Result<Response, CustomError> {
    let description = form.description.trim();
    work_surfaces::update_catalog_image(
        state.sqlx_pool(),
        image_id,
        form.image_ref.trim(),
        if description.is_empty() {
            None
        } else {
            Some(description)
        },
    )
    .await?;
    let note = push_surfaces_best_effort(&state).await.unwrap_or_default();
    Ok(redirect_with(&format!("Catalog entry updated.{note}")))
}

async fn catalog_delete(
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Path(image_id): Path<Uuid>,
) -> Result<Response, CustomError> {
    work_surfaces::delete_catalog_image(state.sqlx_pool(), image_id).await?;
    let note = push_surfaces_best_effort(&state).await.unwrap_or_default();
    Ok(redirect_with(&format!("Catalog entry deleted.{note}")))
}

async fn catalog_default(
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Path(image_id): Path<Uuid>,
) -> Result<Response, CustomError> {
    work_surfaces::set_default_catalog_image(state.sqlx_pool(), image_id).await?;
    let note = push_surfaces_best_effort(&state).await.unwrap_or_default();
    Ok(redirect_with(&format!("Default image updated.{note}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_backend::Backend;
    use crate::config::Config;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use axum::routing::get as get_route;
    use axum_login::{permission_required, AuthnBackend};
    use minijinja::Environment;
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;
    use tower::ServiceExt;
    use tower_sessions_sqlx_store::PostgresStore;

    async fn test_pool() -> Option<sqlx::PgPool> {
        dotenvy::dotenv().ok();
        let url = std::env::var("DATABASE_URL").ok()?;
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .ok()?;
        sqlx::migrate!("../../migrations")
            .set_ignore_missing(true)
            .run(&pool)
            .await
            .ok()?;
        Some(pool)
    }

    async fn test_login(
        axum::extract::Path(user_id): axum::extract::Path<i32>,
        mut auth_session: AuthSession,
    ) -> impl IntoResponse {
        let user = auth_session
            .backend
            .get_user(&user_id)
            .await
            .expect("load login user")
            .expect("login user exists");
        auth_session.login(&user).await.expect("login");
        StatusCode::OK
    }

    /// Mirrors the lib.rs admin nest: the "admin" permission guards the tree.
    async fn test_app(pool: sqlx::PgPool) -> axum::Router {
        let store = PostgresStore::new(pool.clone());
        store.migrate().await.expect("session store migration");
        let state = crate::web::AppState::test_with_template_env(
            pool.clone(),
            Environment::new(),
            Arc::new(Config::test_stub()),
        );
        axum::Router::new()
            .nest("/admin", router())
            .route_layer(permission_required!(Backend, login_url = "/login", "admin"))
            .route("/test-login/{user_id}", get_route(test_login))
            .with_state(state)
            .layer(
                axum_login::AuthManagerLayerBuilder::new(
                    Backend::new(pool),
                    axum_login::tower_sessions::SessionManagerLayer::new(store),
                )
                .build(),
            )
    }

    async fn seed_user(pool: &sqlx::PgPool, is_admin: bool) -> i32 {
        let unique = Uuid::new_v4().simple().to_string();
        sqlx::query_scalar::<_, i32>(
            "INSERT INTO users (email, username, display_name, passhash, is_admin)
             VALUES ($1, $2, $3, 'x', $4) RETURNING id",
        )
        .bind(format!("sbx-admin-{unique}@example.test"))
        .bind(format!("sa{}", &unique[..28]))
        .bind("Sandbox Admin Test")
        .bind(is_admin)
        .fetch_one(pool)
        .await
        .expect("seed user")
    }

    async fn login_cookie(app: &axum::Router, user_id: i32) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/test-login/{user_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("login response");
        assert_eq!(response.status(), StatusCode::OK);
        response
            .headers()
            .get(header::SET_COOKIE)
            .expect("session cookie")
            .to_str()
            .expect("cookie str")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_string()
    }

    async fn post_form(
        app: &axum::Router,
        cookie: &str,
        uri: &str,
        body: String,
    ) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .expect("form response")
    }

    #[tokio::test]
    async fn catalog_crud_is_admin_gated() {
        let Some(pool) = test_pool().await else {
            eprintln!("skipping admin sandbox test: DATABASE_URL unavailable");
            return;
        };
        let app = test_app(pool.clone()).await;
        let admin = seed_user(&pool, true).await;
        let regular = seed_user(&pool, false).await;
        let admin_cookie = login_cookie(&app, admin).await;
        let regular_cookie = login_cookie(&app, regular).await;

        let unique = Uuid::new_v4().simple().to_string();
        let name = format!("adm-img-{}", &unique[..12]);
        let body = format!("name={name}&image_ref=example.invalid%2Fimg%3Alatest&description=t");

        // Non-admin is bounced by the nest guard before any handler runs.
        let response = post_form(&app, &regular_cookie, "/admin/sandbox/catalog", body.clone()).await;
        assert_ne!(response.status(), StatusCode::SEE_OTHER);

        // Admin creates, sets default, deletes.
        let response = post_form(&app, &admin_cookie, "/admin/sandbox/catalog", body).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let (image_id,): (Uuid,) = sqlx::query_as(
            "SELECT id FROM sandbox_catalog_images WHERE name = $1",
        )
        .bind(&name)
        .fetch_one(&pool)
        .await
        .expect("catalog row");
        let response = post_form(
            &app,
            &admin_cookie,
            &format!("/admin/sandbox/catalog/{image_id}/delete"),
            String::new(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }
}
