    use super::*;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use minijinja::Environment;
    use sqlx::{postgres::PgPoolOptions, types::Json};
    use std::sync::Arc;
    use tower::ServiceExt;
    use tower_sessions_sqlx_store::PostgresStore;

    use crate::{config::Config, web::AppState};

    static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn test_pool() -> Option<sqlx::PgPool> {
        dotenvy::dotenv().ok();
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("skipping DB-backed admin route test: DATABASE_URL is not set");
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
                    "skipping DB-backed admin route test: could not connect to DATABASE_URL: {err}"
                );
                return None;
            }
        };
        if let Err(err) = sqlx::migrate!("../../migrations")
            .set_ignore_missing(true)
            .run(&pool)
            .await
        {
            eprintln!("skipping DB-backed admin route test: migrations failed: {err}");
            return None;
        }
        Some(pool)
    }

    fn test_state(pool: sqlx::PgPool) -> AppState {
        let config = Arc::new(Config::test_stub());
        let mut template_env = Environment::new();
        template_env
            .add_template("admin/bears/detail.html", "{{ web_message }} {{ web_sources | length }} {{ web_approvals | length }} {{ web_fetches | length }}{% for approval in web_approvals %} {{ approval.approved_by_user_label }}{% endfor %}")
            .expect("add test template");
        AppState::test_with_template_env(pool, template_env, config)
    }

    async fn test_app(pool: sqlx::PgPool) -> axum::Router {
        let store = PostgresStore::new(pool.clone());
        store.migrate().await.expect("session store migration");
        Router::new()
            .merge(router())
            .with_state(test_state(pool.clone()))
            .layer(
                axum_login::AuthManagerLayerBuilder::new(
                    crate::auth_backend::Backend::new(pool),
                    axum_login::tower_sessions::SessionManagerLayer::new(store),
                )
                .build(),
            )
    }

    async fn create_test_bear(pool: &sqlx::PgPool) -> Uuid {
        bears_db::create_bear(
            pool,
            BearParams {
                slug: &format!("web-admin-{}", Uuid::new_v4()),
                name: "Web Admin Test Bear",
                description: "",
                system_prompt: "System prompt",
                default_model: None,
                tools_enabled: None::<Json<serde_json::Value>>,
                context_profile: None,
            },
        )
        .await
        .expect("create bear")
    }

    async fn create_test_user(pool: &sqlx::PgPool) -> i32 {
        sqlx::query_scalar::<_, i32>(
            r"
            INSERT INTO users (email, username, display_name, passhash, is_admin)
            VALUES ($1, $2, $3, $4, true)
            RETURNING id
            ",
        )
        .bind(format!("web-admin-{}@example.test", Uuid::new_v4()))
        .bind(format!("wa{}", &Uuid::new_v4().simple().to_string()[..28]))
        .bind("Admin Display")
        .bind("test-passhash")
        .fetch_one(pool)
        .await
        .expect("create user")
    }

    #[tokio::test]
    async fn add_web_source_route_normalizes_host_and_flashes() {
        let _guard = TEST_DB_LOCK.lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = create_test_bear(&pool).await;
        let _user_id = create_test_user(&pool).await;
        let app = test_app(pool.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bears/{bear_id}/web-sources"))
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
        let stored: String = sqlx::query_scalar(
            "SELECT scope_value FROM bear_web_sources WHERE bear_id = $1 AND scope_kind = 'host'",
        )
        .bind(bear_id)
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
        let bear_id = create_test_bear(&pool).await;
        let _user_id = create_test_user(&pool).await;
        let app = test_app(pool.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bears/{bear_id}/web-sources"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("scope_kind=host&scope_value=https%3A%2F%2Fexample.com%2Fdocs&policy=preferred&label=&priority=0"))
                    .unwrap(),
            )
            .await
            .expect("validation response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("host must be a bare hostname"));
    }

    #[tokio::test]
    async fn add_and_revoke_web_approval_routes_update_active_approvals() {
        let _guard = TEST_DB_LOCK.lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = create_test_bear(&pool).await;
        let _user_id = create_test_user(&pool).await;
        let app = test_app(pool.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bears/{bear_id}/web-approvals"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("scope_kind=host&scope_value=Docs.RS"))
                    .unwrap(),
            )
            .await
            .expect("add approval response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let approval_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM bear_web_approvals WHERE bear_id = $1 AND scope_value = 'docs.rs' AND revoked_at IS NULL",
        )
        .bind(bear_id)
        .fetch_one(&pool)
        .await
        .expect("active approval");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/bears/{bear_id}/web-approvals/{approval_id}/revoke"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("revoke response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::bigint FROM bear_web_approvals WHERE bear_id = $1 AND revoked_at IS NULL",
        )
        .bind(bear_id)
        .fetch_one(&pool)
        .await
        .expect("approval count");
        assert_eq!(active_count, 0);
    }

    #[tokio::test]
    async fn detail_route_displays_approval_user_label_and_recent_fetches() {
        let _guard = TEST_DB_LOCK.lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = create_test_bear(&pool).await;
        let user_id = create_test_user(&pool).await;
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

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/bears/{bear_id}?message=Saved"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("detail response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("Saved"));
        assert!(body.contains("Admin Display"));
        assert!(body.contains("1 1") || body.contains("1 1 1"));
    }
