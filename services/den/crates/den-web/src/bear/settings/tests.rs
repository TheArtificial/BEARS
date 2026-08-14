//! Route tests for the bear web-policy settings actions (web sources /
//! approvals) and the resources (policy) view. Ported from the retired
//! `/bears/{id}` admin UI tests when those routes became redirects; the
//! handlers now live on `/bear/{slug}/…` and require a logged-in bear admin,
//! so each test seeds a user + membership and logs in through a test-only
//! route before exercising the real router.

use super::*;
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::IntoResponse,
    routing::get,
};
use axum_login::AuthnBackend;
use http_body_util::BodyExt;
use minijinja::Environment;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use tower_sessions_sqlx_store::PostgresStore;

use crate::{auth_backend::Backend, config::Config};

static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn parses_tool_budget_multiplier_form_values() {
    assert_eq!(parse_tool_budget_multiplier_form_value("").unwrap(), None);
    assert_eq!(
        parse_tool_budget_multiplier_form_value("inherit").unwrap(),
        None
    );
    assert_eq!(
        parse_tool_budget_multiplier_form_value("1.5").unwrap(),
        Some(1.5)
    );
    assert!(parse_tool_budget_multiplier_form_value("0").is_err());
    assert!(parse_tool_budget_multiplier_form_value("11").is_err());
}

async fn test_pool() -> Option<sqlx::PgPool> {
    dotenvy::dotenv().ok();
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping DB-backed settings route test: DATABASE_URL is not set");
        return None;
    };
    let pool = match PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!(
                "skipping DB-backed settings route test: could not connect to DATABASE_URL: {err}"
            );
            return None;
        }
    };
    if let Err(err) = sqlx::migrate!("../../migrations")
        .set_ignore_missing(true)
        .run(&pool)
        .await
    {
        eprintln!("skipping DB-backed settings route test: migrations failed: {err}");
        return None;
    }
    Some(pool)
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    let config = Arc::new(Config::test_stub());
    let mut template_env = Environment::new();
    template_env
            .add_template("bear/settings/policy.html", "{{ message }} {{ web_sources | length }} {{ web_approvals | length }} {{ web_fetches | length }}{% for approval in web_approvals %} {{ approval.approved_by_user_label }}{% endfor %}")
            .expect("add test template");
    AppState::test_with_template_env(pool, template_env, config)
}

/// Test-only login endpoint: establishes an axum-login session for the given
/// user id so requests carrying the returned cookie hit the real handlers as
/// that user.
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

async fn test_app(pool: sqlx::PgPool) -> axum::Router {
    let store = PostgresStore::new(pool.clone());
    store.migrate().await.expect("session store migration");
    Router::new()
        .merge(router())
        .route("/test-login/{user_id}", get(test_login))
        .with_state(test_state(pool.clone()))
        .layer(
            axum_login::AuthManagerLayerBuilder::new(
                Backend::new(pool),
                axum_login::tower_sessions::SessionManagerLayer::new(store),
            )
            .build(),
        )
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
        .expect("session cookie set on login")
        .to_str()
        .expect("cookie is valid string")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string()
}

async fn create_test_bear(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    bears_db::create_bear(
        pool,
        bears_db::BearParams {
            slug,
            name: "Web Settings Test Bear",
            description: "",
            system_prompt: "System prompt",
            default_model: None,
            tools_enabled: None::<sqlx::types::Json<serde_json::Value>>,
            context_profile: None,
        },
    )
    .await
    .expect("create bear")
}

/// User with a verified email (the settings pages redirect unverified users)
/// and an admin membership on the given bear.
async fn create_bear_admin_user(pool: &sqlx::PgPool, bear_id: Uuid) -> i32 {
    let unique = Uuid::new_v4().simple().to_string();
    let email = format!("web-settings-{unique}@example.test");
    let username = format!("ws{}", &unique[..28]);
    let user_id = sqlx::query_scalar!(
        r#"
            INSERT INTO users (email, username, display_name, passhash)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        email,
        username,
        "Admin Display",
        "test-passhash"
    )
    .fetch_one(pool)
    .await
    .expect("create user");
    sqlx::query!(
        r#"
            INSERT INTO email_configs (user_id, email_address, active, verified_at)
            VALUES ($1, $2, true, now())
            "#,
        user_id,
        format!("web-settings-{unique}@example.test")
    )
    .execute(pool)
    .await
    .expect("verify email");
    bears_db::grant_membership(pool, user_id, bear_id, Some(BEAR_ROLE_ADMIN))
        .await
        .expect("grant admin membership");
    user_id
}

fn fresh_slug() -> String {
    format!("web-settings-{}", Uuid::new_v4())
}

#[tokio::test]
async fn add_web_source_route_normalizes_host_and_flashes() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let slug = fresh_slug();
    let bear_id = create_test_bear(&pool, &slug).await;
    let user_id = create_bear_admin_user(&pool, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bear/{slug}/web-sources"))
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("scope_kind=host&scope_value=Example.COM%3A8443.&policy=preferred&label=Docs&priority=10"))
                    .unwrap(),
            )
            .await
            .expect("add source response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .contains("message=Web%20source%20saved"));
    let stored: String = sqlx::query_scalar!(
        "SELECT scope_value FROM bear_web_sources WHERE bear_id = $1 AND scope_kind = 'host'",
        bear_id
    )
    .fetch_one(&pool)
    .await
    .expect("stored source");
    assert_eq!(stored, "example.com:8443");
}

#[tokio::test]
async fn add_web_source_route_rejects_url_in_host_scope() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let slug = fresh_slug();
    let bear_id = create_test_bear(&pool, &slug).await;
    let user_id = create_bear_admin_user(&pool, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bear/{slug}/web-sources"))
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("scope_kind=host&scope_value=https%3A%2F%2Fexample.com%2Fdocs&policy=preferred&label=&priority=0"))
                    .unwrap(),
            )
            .await
            .expect("validation response");

    // Invalid input flashes the normalization error back to the resources page.
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let location = urlencoding::decode(&location).expect("decode location");
    assert!(
        location.contains("host must be a bare hostname"),
        "unexpected redirect: {location}"
    );
    let stored: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*)::bigint AS \"count!: i64\" FROM bear_web_sources WHERE bear_id = $1",
        bear_id
    )
    .fetch_one(&pool)
    .await
    .expect("source count");
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn add_and_revoke_web_approval_routes_update_active_approvals() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let slug = fresh_slug();
    let bear_id = create_test_bear(&pool, &slug).await;
    let user_id = create_bear_admin_user(&pool, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/bear/{slug}/web-approvals"))
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("scope_kind=host&scope_value=Docs.RS"))
                .unwrap(),
        )
        .await
        .expect("add approval response");
    let status = response.status();
    if status != StatusCode::SEE_OTHER {
        let body = response.into_body().collect().await.unwrap().to_bytes();
        panic!(
            "add approval: expected 303, got {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }

    let approval_id: Uuid = sqlx::query_scalar!(
        "SELECT id FROM bear_web_approvals WHERE bear_id = $1 AND scope_value = 'docs.rs' AND revoked_at IS NULL",
        bear_id
    )
    .fetch_one(&pool)
        .await
        .expect("active approval");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/bear/{slug}/web-approvals/{approval_id}/revoke"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("revoke response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let active_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*)::bigint AS \"count!: i64\" FROM bear_web_approvals WHERE bear_id = $1 AND revoked_at IS NULL",
        bear_id
    )
    .fetch_one(&pool)
    .await
    .expect("approval count");
    assert_eq!(active_count, 0);
}

#[tokio::test]
async fn resources_view_displays_approval_user_label_and_recent_fetches() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let slug = fresh_slug();
    let bear_id = create_test_bear(&pool, &slug).await;
    let user_id = create_bear_admin_user(&pool, bear_id).await;
    web_policy::record_web_approval(
        &pool,
        bear_id,
        "host",
        "example.com",
        Some(user_id),
        "admin",
        None,
    )
    .await
    .expect("record approval");
    web_policy::record_web_fetch_attempt(
        &pool,
        web_policy::WebFetchAuditParams {
            bear_id,
            session_id: Some("session-1"),
            tool_call_id: Some("tool-1"),
            url: "https://example.com/",
            final_url: None,
            host: "example.com",
            execution_location: "den",
            approval_kind: "user_host",
            http_status: Some(200),
            content_type: Some("text/html"),
            bytes: Some(123),
        },
    )
    .await
    .expect("record fetch");

    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/bear/{slug}/resources?message=Saved"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("resources response");
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "resources view: {}",
        String::from_utf8_lossy(&body)
    );
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("Saved"));
    assert!(body.contains("Admin Display"));
    assert!(body.contains("0 1 1"));
}
