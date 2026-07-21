//! Route tests for the work UI's job-creation and dispatch forms. Postgres-
//! backed (skip without DATABASE_URL); login is seeded through a test-only
//! route, same pattern as `bear::settings::tests`.

use super::*;
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    routing::get,
};
use axum_login::AuthnBackend;
use minijinja::Environment;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use tower_sessions_sqlx_store::PostgresStore;

use crate::{auth_backend::Backend, config::Config};

static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn test_pool() -> Option<sqlx::PgPool> {
    dotenvy::dotenv().ok();
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping DB-backed work route test: DATABASE_URL is not set");
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
            eprintln!("skipping DB-backed work route test: could not connect: {err}");
            return None;
        }
    };
    if let Err(err) = sqlx::migrate!("../../migrations")
        .set_ignore_missing(true)
        .run(&pool)
        .await
    {
        eprintln!("skipping DB-backed work route test: migrations failed: {err}");
        return None;
    }
    Some(pool)
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    let config = Arc::new(Config::test_stub());
    let template_env = Environment::new();
    AppState::test_with_template_env(pool, template_env, config)
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
        .expect("cookie str")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string()
}

/// Member user + bear (admin membership) for the work UI's scoping checks.
async fn seed_member(pool: &sqlx::PgPool) -> (i32, Uuid) {
    let unique = Uuid::new_v4().simple().to_string();
    let user_id = sqlx::query_scalar::<_, i32>(
        "INSERT INTO users (email, username, display_name, passhash)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(format!("work-ui-{unique}@example.test"))
    .bind(format!("wu{}", &unique[..28]))
    .bind("Work UI Test")
    .bind("test-passhash")
    .fetch_one(pool)
    .await
    .expect("create user");
    let bear_id = bears_db::create_bear(
        pool,
        bears_db::BearParams {
            slug: &format!("work-ui-{}", &unique[..12]),
            name: "Work UI Test Bear",
            description: "",
            system_prompt: "",
            default_model: None,
            tools_enabled: None::<sqlx::types::Json<serde_json::Value>>,
            context_profile: None,
        },
    )
    .await
    .expect("create bear");
    bears_db::grant_membership(pool, user_id, bear_id, Some("admin"))
        .await
        .expect("grant membership");
    (user_id, bear_id)
}

#[tokio::test]
async fn create_job_form_creates_work_job_with_tasks() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id) = seed_member(&pool).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let body = format!(
        "bear_id={bear_id}&goal=Ship+the+site&root=site&commit_policy=per_task\
         &work_branch=&task_title=Update+headline&task_criteria=headline+mentions+bears\
         &task_title=&task_criteria="
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/work/new")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("create job response");
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if status != StatusCode::SEE_OTHER {
        use http_body_util::BodyExt;
        let body = response.into_body().collect().await.unwrap().to_bytes();
        panic!(
            "create job: expected 303, got {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    let job_id: Uuid = location
        .rsplit('/')
        .next()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .expect("redirect to job page");

    let (goal, surface, policy, branch): (String, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT goal, work_surface_ref, commit_policy, work_branch
             FROM bear_jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .expect("job row");
    assert_eq!(goal, "Ship the site");
    assert_eq!(surface.as_deref(), Some("site"));
    assert_eq!(policy.as_deref(), Some("per_task"));
    assert!(branch.is_none(), "blank branch stays unset until dispatch");

    // Exactly one non-blank task, assigned to work, with the criterion.
    let tasks: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT title, assigned_to_role FROM bear_tasks WHERE job_id = $1")
            .bind(job_id)
            .fetch_all(&pool)
            .await
            .expect("tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].0, "Update headline");
    assert_eq!(tasks[0].1.as_deref(), Some("work"));

    let response = post_form(
        &app,
        &cookie,
        &format!("/work/jobs/{job_id}/edit"),
        "goal=Ship+the+updated+site&surface_id=&commit_policy=per_job&work_branch=feature%2Fupdated"
            .to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let (goal, surface, policy, branch): (String, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT goal, work_surface_ref, commit_policy, work_branch FROM bear_jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .expect("edited job row");
    assert_eq!(goal, "Ship the updated site");
    assert!(surface.is_none());
    assert_eq!(policy.as_deref(), Some("per_job"));
    assert_eq!(branch.as_deref(), Some("feature/updated"));
}

#[tokio::test]
async fn duplicate_job_copies_definition_and_resets_execution_state() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id) = seed_member(&pool).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let response = post_form(
        &app,
        &cookie,
        "/work/new",
        format!(
            "bear_id={bear_id}&goal=Reusable+job&root=site&commit_policy=per_task\
             &work_branch=&task_title=Build+artifact&task_criteria=artifact+exists%3Btests+pass"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let source_id = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|location| location.rsplit('/').next())
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("source job redirect");
    sqlx::query("UPDATE bear_jobs SET work_branch = 'feature/original' WHERE id = $1")
        .bind(source_id)
        .execute(&pool)
        .await
        .expect("set source branch");
    let source_run_id: Uuid =
        sqlx::query_scalar("SELECT current_run_id FROM bear_jobs WHERE id = $1")
            .bind(source_id)
            .fetch_one(&pool)
            .await
            .expect("source run id");

    let response = post_form(
        &app,
        &cookie,
        &format!("/work/jobs/{source_id}/duplicate"),
        String::new(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let duplicate_id = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|location| location.rsplit('/').next())
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("duplicate job redirect");
    assert_ne!(duplicate_id, source_id);

    let (goal, surface, policy, branch, status, current_run): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<Uuid>,
    ) = sqlx::query_as(
        "SELECT goal, work_surface_ref, commit_policy, work_branch, status, current_run_id \
         FROM bear_jobs WHERE id = $1",
    )
    .bind(duplicate_id)
    .fetch_one(&pool)
    .await
    .expect("duplicate job row");
    assert_eq!(goal, "Reusable job (copy)");
    assert_eq!(surface.as_deref(), Some("site"));
    assert_eq!(policy.as_deref(), Some("per_task"));
    assert!(branch.is_none());
    assert_eq!(status, "ready");
    let duplicate_run_id = current_run.expect("fresh duplicate run");
    assert_ne!(duplicate_run_id, source_run_id);

    let tasks: Vec<(
        String,
        String,
        sqlx::types::Json<Vec<String>>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT title, body, completion_criteria, assigned_to_role \
             FROM bear_tasks WHERE job_id = $1 ORDER BY sibling_order",
    )
    .bind(duplicate_id)
    .fetch_all(&pool)
    .await
    .expect("duplicate tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].0, "Build artifact");
    assert_eq!(tasks[0].1, "Build artifact");
    assert_eq!(tasks[0].2 .0, vec!["artifact exists", "tests pass"]);
    assert_eq!(tasks[0].3.as_deref(), Some("work"));
    let task_statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM bear_task_run_state WHERE run_id = $1")
            .bind(duplicate_run_id)
            .fetch_all(&pool)
            .await
            .expect("duplicate task states");
    assert_eq!(task_statuses, vec!["pending"]);
}

#[tokio::test]
async fn job_lifecycle_can_extend_then_complete() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id) = seed_member(&pool).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;
    let response = post_form(
        &app,
        &cookie,
        "/work/new",
        format!(
            "bear_id={bear_id}&goal=Lifecycle+job&root=&commit_policy=propose_only\
             &task_title=First+task&task_criteria=first+done"
        ),
    )
    .await;
    let job_id = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|location| location.rsplit('/').next())
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("job redirect");
    let run_id: Uuid = sqlx::query_scalar("SELECT current_run_id FROM bear_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .expect("current run");

    let response = post_form(
        &app,
        &cookie,
        &format!("/work/jobs/{job_id}/extend"),
        "title=Second+task&body=&criteria=second+done".to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let task_count: i64 = sqlx::query_scalar("SELECT count(*) FROM bear_tasks WHERE job_id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .expect("task count");
    assert_eq!(task_count, 2);
    sqlx::query("UPDATE bear_task_run_state SET status = 'done' WHERE run_id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("complete task states");

    let response = post_form(
        &app,
        &cookie,
        &format!("/work/jobs/{job_id}/complete"),
        String::new(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let status: String = sqlx::query_scalar("SELECT status FROM bear_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .expect("job status");
    let run_state: String = sqlx::query_scalar("SELECT state FROM bear_job_runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(&pool)
        .await
        .expect("run state");
    let criterion_statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM bear_job_criteria_state WHERE run_id = $1")
            .bind(run_id)
            .fetch_all(&pool)
            .await
            .expect("criterion states");
    assert_eq!(status, "completed");
    assert_eq!(run_state, "completed");
    assert!(criterion_statuses.iter().all(|status| status == "met"));
}

#[tokio::test]
async fn job_scoped_surface_creation_assigns_and_attaches_surface() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id) = seed_member(&pool).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;
    let response = post_form(
        &app,
        &cookie,
        "/work/new",
        format!(
            "bear_id={bear_id}&goal=Surface+job&root=&commit_policy=propose_only\
             &task_title=Use+repo&task_criteria=repo+used"
        ),
    )
    .await;
    let job_id = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|location| location.rsplit('/').next())
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("job redirect");
    let surface_name = format!("job-surface-{}", &Uuid::new_v4().simple().to_string()[..12]);
    let response = post_form(
        &app,
        &cookie,
        "/work/surfaces/new",
        format!(
            "name={surface_name}&description=&upstream_url=https%3A%2F%2Fexample.invalid%2Frepo.git\
             &default_ref=main&default_image=&credential_kind=&credential_value=\
             &bear_id={bear_id}&return_job_id={job_id}"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let redirect = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let (surface_ref, surface_id): (Option<String>, Option<Uuid>) =
        sqlx::query_as("SELECT work_surface_ref, work_surface_id FROM bear_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("attached job surface");
    assert_eq!(surface_ref.as_deref(), Some(surface_name.as_str()));
    let surface_id = surface_id.expect("surface id");
    assert!(redirect.starts_with(&format!("/work/surfaces/{surface_id}?message=")));
    assert!(redirect.contains("not%20ready"));
    let assignment_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM work_surface_bears WHERE surface_id = $1 AND bear_id = $2",
    )
    .bind(surface_id)
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("surface assignment");
    assert_eq!(assignment_count, 1);

    let response = post_form(
        &app,
        &cookie,
        &format!("/work/jobs/{job_id}/edit"),
        format!("goal=Surface+job&surface_id={surface_id}&commit_policy=per_task&work_branch=main"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = post_form(
        &app,
        &cookie,
        &format!("/work/jobs/{job_id}/edit"),
        format!(
            "goal=Surface+job&surface_id={surface_id}&commit_policy=per_task&work_branch=&allow_default_ref=true"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let branch: Option<String> =
        sqlx::query_scalar("SELECT work_branch FROM bear_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("default work branch");
    assert_eq!(branch.as_deref(), Some("main"));
}

#[tokio::test]
async fn dispatch_form_enqueues_run_with_root_and_image() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id) = seed_member(&pool).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    // Create the job through the same form, then dispatch its task.
    let body = format!(
        "bear_id={bear_id}&goal=Dispatch+me&root=&commit_policy=propose_only\
         &task_title=Do+the+thing&task_criteria=thing+is+done"
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/work/new")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("create job response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let (task_id,): (Uuid,) =
        sqlx::query_as("SELECT id FROM bear_tasks WHERE bear_id = $1 AND title = 'Do the thing'")
            .bind(bear_id)
            .fetch_one(&pool)
            .await
            .expect("task id");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/work/tasks/{task_id}/dispatch"))
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("root=site&image=rust&git_ref="))
                .unwrap(),
        )
        .await
        .expect("dispatch response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let (root, image, git_ref): (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT root_name, image_name, git_ref FROM bear_work_runs
         WHERE task_id = $1 AND state = 'queued'",
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .expect("queued run");
    assert_eq!(root.as_deref(), Some("site"));
    assert_eq!(image.as_deref(), Some("rust"));
    assert!(git_ref.is_none(), "blank git_ref stays unset");
}

/// Helper: POST a form to the app with the session cookie; returns the
/// response.
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
async fn surface_management_is_owner_scoped_and_grantable() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (owner_id, _bear_id) = seed_member(&pool).await;
    let (other_id, _other_bear) = seed_member(&pool).await;
    let app = test_app(pool.clone()).await;
    let owner_cookie = login_cookie(&app, owner_id).await;
    let other_cookie = login_cookie(&app, other_id).await;

    let unique = Uuid::new_v4().simple().to_string();
    let name = format!("ui-surface-{}", &unique[..12]);
    let response = post_form(
        &app,
        &owner_cookie,
        "/work/surfaces/new",
        format!(
            "name={name}&description=&upstream_url=https%3A%2F%2Fexample.invalid%2Frepo.git\
             &default_ref=main&default_image=&credential_kind=&credential_value="
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let (surface_id, created_by): (Uuid, i32) =
        sqlx::query_as("SELECT id, created_by_user_id FROM work_surfaces WHERE name = $1")
            .bind(&name)
            .fetch_one(&pool)
            .await
            .expect("surface row");
    assert_eq!(created_by, owner_id);

    // Non-manager: manage page and mutations deny as 404.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/work/surfaces/{surface_id}"))
                .header(header::COOKIE, &other_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = post_form(
        &app,
        &other_cookie,
        &format!("/work/surfaces/{surface_id}/update"),
        "description=x&upstream_url=https%3A%2F%2Fevil.invalid%2Fr.git&default_ref=main&default_image=".to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Owner grants the other user; the grantee can now update.
    let other_username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(other_id)
        .fetch_one(&pool)
        .await
        .expect("username");
    let response = post_form(
        &app,
        &owner_cookie,
        &format!("/work/surfaces/{surface_id}/managers/grant"),
        format!("username={other_username}&role=manager"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let response = post_form(
        &app,
        &other_cookie,
        &format!("/work/surfaces/{surface_id}/update"),
        "description=updated+by+manager&upstream_url=https%3A%2F%2Fexample.invalid%2Frepo.git&default_ref=trunk&default_image=".to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let (default_ref,): (String,) =
        sqlx::query_as("SELECT default_ref FROM work_surfaces WHERE id = $1")
            .bind(surface_id)
            .fetch_one(&pool)
            .await
            .expect("updated row");
    assert_eq!(default_ref, "trunk");
}

#[tokio::test]
async fn create_job_enforces_surface_assignment() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id) = seed_member(&pool).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let unique = Uuid::new_v4().simple().to_string();
    let name = format!("job-surface-{}", &unique[..12]);
    let response = post_form(
        &app,
        &cookie,
        "/work/surfaces/new",
        format!(
            "name={name}&description=&upstream_url=https%3A%2F%2Fexample.invalid%2Frepo.git\
             &default_ref=main&default_image=&credential_kind=&credential_value="
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let (surface_id,): (Uuid,) = sqlx::query_as("SELECT id FROM work_surfaces WHERE name = $1")
        .bind(&name)
        .fetch_one(&pool)
        .await
        .expect("surface row");

    // The bear is not assigned: job creation with the surface is rejected.
    let job_body = format!(
        "bear_id={bear_id}&goal=Surface+gated&surface_id={surface_id}&root=&commit_policy=per_task\
         &task_title=Do+it&task_criteria=done"
    );
    let response = post_form(&app, &cookie, "/work/new", job_body.clone()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Assign the bear from the surface page, then the same form succeeds and
    // binds both name and id.
    let response = post_form(
        &app,
        &cookie,
        &format!("/work/surfaces/{surface_id}/bears/assign"),
        format!("bear_id={bear_id}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let response = post_form(&app, &cookie, "/work/new", job_body).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let (surface_ref, bound_id): (Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT work_surface_ref, work_surface_id FROM bear_jobs
         WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("job row");
    assert_eq!(surface_ref.as_deref(), Some(name.as_str()));
    assert_eq!(bound_id, Some(surface_id));
}
