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
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;

#[test]
fn cargo_offline_cache_miss_is_the_primary_outcome() {
    let failure = serde_json::json!({
        "code": "cargo_offline_cache_miss",
        "required_package": "serde",
    });
    let run = WorkRunRow {
        id: Uuid::nil(),
        bear_id: Uuid::nil(),
        job_id: Uuid::nil(),
        job_run_id: Uuid::nil(),
        attempt: 1,
        state: "succeeded".into(),
        runner_id: None,
        lease_expires_at: None,
        cancel_requested: false,
        cancel_requested_by: None,
        cancel_reason: None,
        cancel_requested_at: None,
        root_name: None,
        git_ref: None,
        image_name: None,
        sandbox_server_url: None,
        sandbox_id: None,
        sandbox_type: None,
        sandbox_strength: None,
        work_surface: None,
        execution_target: "sandbox".into(),
        attached_client_session_id: None,
        attachment_state: None,
        attachment_warning: None,
        disconnected_at: None,
        disconnect_deadline_at: None,
        bearwire_session_id: None,
        result_summary: Some("headless turn reached a terminal run event".into()),
        result_refs: None,
        usage: None,
        error: None,
        queued_at: time::OffsetDateTime::UNIX_EPOCH,
        started_at: None,
        finished_at: None,
        updated_at: time::OffsetDateTime::UNIX_EPOCH,
    };
    assert_eq!(
        work_run_outcome(&run, &[(Uuid::nil(), "pending".into())], Some(&failure)),
        "Blocked: Rust dependencies are unavailable in the offline cache. `serde` could not be resolved. Dependency preparation was not attempted; prepare Rust dependencies, then retry Cargo.",
    );
}
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
    let mut config = Config::test_stub();
    config.templates_dir = format!("{}/src/templates", env!("CARGO_MANIFEST_DIR"));
    let config = Arc::new(config);
    let template_env = crate::template_environment(config.as_ref());
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
        .nest("/bear/{bear_slug}", docket_router())
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
async fn seed_member(pool: &sqlx::PgPool) -> (i32, Uuid, String) {
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
    let slug = format!("work-ui-{}", &unique[..12]);
    let bear_id = bears_db::create_bear(
        pool,
        bears_db::BearParams {
            slug: &slug,
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
    (user_id, bear_id, slug)
}

async fn assigned_surface_id(pool: &sqlx::PgPool, user_id: i32, bear_id: Uuid) -> Uuid {
    let name = format!("work-ui-{}", Uuid::new_v4().simple());
    let surface_id: Uuid = sqlx::query_scalar(
        "INSERT INTO work_surfaces (name, upstream_url, created_by_user_id)
         VALUES ($1, 'https://example.test/work-ui.git', $2) RETURNING id",
    )
    .bind(name)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("create work surface");
    sqlx::query("INSERT INTO work_surface_bears (surface_id, bear_id) VALUES ($1, $2)")
        .bind(surface_id)
        .bind(bear_id)
        .execute(pool)
        .await
        .expect("assign work surface");
    surface_id
}

#[tokio::test]
async fn create_job_form_creates_work_job_with_tasks() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id, bear_slug) = seed_member(&pool).await;
    let surface_id = assigned_surface_id(&pool, user_id, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let body = format!(
        "bear_id={bear_id}&goal=Ship+the+site&surface_id={surface_id}&commit_policy=per_task\
         &work_branch=&task_title=Update+headline&task_criteria=headline+mentions+bears\
         &task_title=&task_criteria="
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/bear/{bear_slug}/jobs/new"))
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
    assert_eq!(
        location,
        format!(
            "/bear/{bear_slug}/jobs/{}",
            route_id(
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT id FROM bear_jobs WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1"
                )
                .bind(bear_id)
                .fetch_one(&pool)
                .await
                .expect("job id")
            )
        )
    );
    let job_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM bear_jobs WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("job id");

    let (goal, selected_surface_id, policy, branch): (
        String,
        Option<Uuid>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT goal, work_surface_id, commit_policy, work_branch
             FROM bear_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&pool)
    .await
    .expect("job row");
    assert_eq!(goal, "Ship the site");
    assert_eq!(selected_surface_id, Some(surface_id));
    assert_eq!(policy.as_deref(), Some("per_task"));
    assert!(branch.is_none(), "blank branch stays unset until dispatch");

    // Exactly one non-blank task with the criterion.
    let tasks: Vec<String> = sqlx::query_scalar("SELECT title FROM bear_tasks WHERE job_id = $1")
        .bind(job_id)
        .fetch_all(&pool)
        .await
        .expect("tasks");
    assert_eq!(tasks, vec!["Update headline"]);

    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/{}/edit", route_id(job_id)),
        format!("goal=Ship+the+updated+site&surface_id={surface_id}&commit_policy=per_job&work_branch=feature%2Fupdated"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let (goal, selected_surface_id, policy, branch): (
        String,
        Option<Uuid>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT goal, work_surface_id, commit_policy, work_branch FROM bear_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&pool)
    .await
    .expect("edited job row");
    assert_eq!(goal, "Ship the updated site");
    assert_eq!(selected_surface_id, Some(surface_id));
    assert_eq!(policy.as_deref(), Some("per_job"));
    assert_eq!(branch.as_deref(), Some("feature/updated"));
}

#[tokio::test]
async fn work_dashboard_hides_completed_jobs_until_requested() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id, bear_slug) = seed_member(&pool).await;
    let surface_id = assigned_surface_id(&pool, user_id, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;
    let unique = Uuid::new_v4().simple().to_string();
    let active_goal = format!("Active dashboard job {unique}");
    let completed_goal = format!("Completed dashboard job {unique}");

    for goal in [&active_goal, &completed_goal] {
        let response = post_form(
            &app,
            &cookie,
            &format!("/bear/{bear_slug}/jobs/new"),
            format!(
                "bear_id={bear_id}&goal={}&surface_id={surface_id}&commit_policy=none&work_branch=&task_title=Check&task_criteria=done",
                urlencoding::encode(goal)
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }
    sqlx::query("UPDATE bear_jobs SET status = 'completed' WHERE bear_id = $1 AND goal = $2")
        .bind(bear_id)
        .bind(&completed_goal)
        .execute(&pool)
        .await
        .expect("complete dashboard job");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/bear/{bear_slug}/jobs"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("dashboard response");
    let status = response.status();
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .into_owned();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains(&active_goal));
    assert!(!body.contains(&completed_goal));
    assert!(body.contains("Show completed jobs"));
    assert!(body.contains(&format!("/bear/{bear_slug}/jobs/new")));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/bear/{bear_slug}/jobs?completed=show"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("completed dashboard response");
    let body = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .into_owned();
    assert!(body.contains(&active_goal));
    assert!(body.contains(&completed_goal));
    assert!(body.contains("Hide completed jobs"));
}

#[tokio::test]
async fn duplicate_job_copies_definition_and_resets_execution_state() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id, bear_slug) = seed_member(&pool).await;
    let surface_id = assigned_surface_id(&pool, user_id, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/new"),
        format!(
            "bear_id={bear_id}&goal=Reusable+job&surface_id={surface_id}&commit_policy=per_task\
             &work_branch=&task_title=Build+artifact&task_criteria=artifact+exists%3Btests+pass"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let source_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM bear_jobs WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("source job");
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
        &format!("/bear/{bear_slug}/jobs/{}/duplicate", route_id(source_id)),
        String::new(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let duplicate_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM bear_jobs WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("duplicate job");
    assert_ne!(duplicate_id, source_id);

    let (goal, duplicate_surface_id, policy, branch, status, current_run): (
        String,
        Option<Uuid>,
        Option<String>,
        Option<String>,
        String,
        Option<Uuid>,
    ) = sqlx::query_as(
        "SELECT goal, work_surface_id, commit_policy, work_branch, status, current_run_id \
         FROM bear_jobs WHERE id = $1",
    )
    .bind(duplicate_id)
    .fetch_one(&pool)
    .await
    .expect("duplicate job row");
    assert_eq!(goal, "Reusable job (copy)");
    assert_eq!(duplicate_surface_id, Some(surface_id));
    assert_eq!(policy.as_deref(), Some("per_task"));
    assert!(branch.is_none());
    assert_eq!(status, "ready");
    let duplicate_run_id = current_run.expect("fresh duplicate run");
    assert_ne!(duplicate_run_id, source_run_id);

    let tasks: Vec<(String, String, sqlx::types::Json<Vec<String>>)> = sqlx::query_as(
        "SELECT title, body, completion_criteria \
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
    let task_statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM bear_task_run_state WHERE run_id = $1")
            .bind(duplicate_run_id)
            .fetch_all(&pool)
            .await
            .expect("duplicate task states");
    assert_eq!(task_statuses, vec!["pending"]);
}

#[tokio::test]
async fn task_tree_can_add_children_and_reorder_siblings() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id, bear_slug) = seed_member(&pool).await;
    let surface_id = assigned_surface_id(&pool, user_id, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/new"),
        format!(
            "bear_id={bear_id}&goal=Edit+the+tree&surface_id={surface_id}&commit_policy=none\
             &task_title=First+root&task_criteria=first+done"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let job_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM bear_jobs WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("job id");
    let first_root_id: Uuid =
        sqlx::query_scalar("SELECT id FROM bear_tasks WHERE job_id = $1 AND title = 'First root'")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("first root task");

    let response = post_form(
        &app,
        &cookie,
        &format!(
            "/bear/{bear_slug}/jobs/{}/tasks/{}/children",
            route_id(job_id),
            route_id(first_root_id)
        ),
        "title=First+child&criteria=child+done&body=".to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let child: (Option<Uuid>, i32) = sqlx::query_as(
        "SELECT parent_task_id, sibling_order FROM bear_tasks WHERE job_id = $1 AND title = 'First child'",
    )
    .bind(job_id)
    .fetch_one(&pool)
    .await
    .expect("child task");
    assert_eq!(child, (Some(first_root_id), 0));

    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/{}/tasks", route_id(job_id)),
        "title=Second+root&body=&criteria=second+done".to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let second_root_id: Uuid =
        sqlx::query_scalar("SELECT id FROM bear_tasks WHERE job_id = $1 AND title = 'Second root'")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("second root task");

    let response = post_form(
        &app,
        &cookie,
        &format!(
            "/bear/{bear_slug}/jobs/{}/tasks/{}/move/up",
            route_id(job_id),
            route_id(second_root_id)
        ),
        String::new(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let roots: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM bear_tasks WHERE job_id = $1 AND parent_task_id IS NULL ORDER BY sibling_order",
    )
    .bind(job_id)
    .fetch_all(&pool)
    .await
    .expect("root ordering");
    assert_eq!(roots, vec![second_root_id, first_root_id]);
}

#[tokio::test]
async fn legacy_job_lifecycle_can_extend_then_complete() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id, bear_slug) = seed_member(&pool).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;
    let surface_name = format!("lifecycle-{}", &Uuid::new_v4().simple().to_string()[..12]);
    let response = post_form(
        &app,
        &cookie,
        "/work/surfaces/new",
        format!(
            "name={surface_name}&description=&upstream_url=https%3A%2F%2Fexample.invalid%2Frepo.git\
             &default_ref=main&default_image=&credential_kind=&credential_value=&bear_id={bear_id}"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let surface_id: Uuid = sqlx::query_scalar("SELECT id FROM work_surfaces WHERE name = $1")
        .bind(&surface_name)
        .fetch_one(&pool)
        .await
        .expect("surface id");

    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/new"),
        format!(
            "bear_id={bear_id}&goal=Lifecycle+job&surface_id={surface_id}&root=&commit_policy=none\
             &task_title=First+task&task_criteria=first+done"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let job_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM bear_jobs WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("job id");
    let run_id: Uuid = sqlx::query_scalar("SELECT current_run_id FROM bear_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .expect("current run");

    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/{}/tasks", route_id(job_id)),
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
        &format!("/bear/{bear_slug}/jobs/{}/complete", route_id(job_id)),
        String::new(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let status: String = sqlx::query_scalar("SELECT status FROM bear_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .expect("job status");
    let bound_surface_id: Option<Uuid> =
        sqlx::query_scalar("SELECT work_surface_id FROM bear_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("job surface binding");
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
    assert_eq!(bound_surface_id, Some(surface_id));
    assert_eq!(run_state, "completed");
    assert!(criterion_statuses.iter().all(|status| status == "met"));
}

#[tokio::test]
async fn job_scoped_surface_creation_assigns_and_attaches_surface() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (user_id, bear_id, bear_slug) = seed_member(&pool).await;
    let initial_surface_id = assigned_surface_id(&pool, user_id, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;
    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/new"),
        format!(
            "bear_id={bear_id}&goal=Surface+job&surface_id={initial_surface_id}&commit_policy=none\
             &task_title=Use+repo&task_criteria=repo+used"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let job_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM bear_jobs WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("job id");
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
    let surface_id: Option<Uuid> =
        sqlx::query_scalar("SELECT work_surface_id FROM bear_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("attached job surface");
    let surface_id = surface_id.expect("surface id");
    assert!(redirect.starts_with(&format!(
        "/work/surfaces/{}?message=",
        &surface_id.simple().to_string()[..16]
    )));
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
        &format!("/bear/{bear_slug}/jobs/{}/edit", route_id(job_id)),
        format!("goal=Surface+job&surface_id={surface_id}&commit_policy=per_task&work_branch=main"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/{}/edit", route_id(job_id)),
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
    let (user_id, bear_id, bear_slug) = seed_member(&pool).await;
    let surface_id = assigned_surface_id(&pool, user_id, bear_id).await;
    let app = test_app(pool.clone()).await;
    let cookie = login_cookie(&app, user_id).await;

    // Create the job through the same form, then dispatch its task.
    let body = format!(
        "bear_id={bear_id}&goal=Dispatch+me&surface_id={surface_id}&commit_policy=none\
         &task_title=Do+the+thing&task_criteria=thing+is+done\
         &task_title=Do+the+next+thing&task_criteria=next+thing+is+done"
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/bear/{bear_slug}/jobs/new"))
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("create job response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let (_task_id, job_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT id, job_id FROM bear_tasks WHERE bear_id = $1 AND title = 'Do the thing'",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("task id");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/bear/{bear_slug}/jobs/{}/dispatch",
                    route_id(job_id)
                ))
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("root=site&image=rust&git_ref="))
                .unwrap(),
        )
        .await
        .expect("dispatch response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let redirect = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("dispatch redirect");

    type QueuedRunRow = (Uuid, Option<String>, Option<String>, Option<String>);
    let runs: Vec<QueuedRunRow> = sqlx::query_as(
        "SELECT id, root_name, image_name, git_ref FROM bear_work_runs
         WHERE job_id = $1 AND state = 'queued' ORDER BY queued_at",
    )
    .bind(job_id)
    .fetch_all(&pool)
    .await
    .expect("queued job runs");
    assert_eq!(runs.len(), 1);
    let (run_id, root, image, git_ref) = &runs[0];
    assert_eq!(
        redirect,
        format!("/bear/{bear_slug}/jobs/runs/{}", route_id(*run_id))
    );
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

#[test]
fn cargo_registry_network_diagnostic_is_actionable() {
    let diagnostic = run_diagnostic(
        Some("cargo test timed out while Cargo attempted to update the crates.io index"),
        None,
        "spurious network error: TLS transfer failed",
    )
    .expect("Cargo registry failure is recognized");
    assert_eq!(diagnostic.title, "Cargo dependency access failed");
    assert!(diagnostic.recovery.contains("retry the blocked task"));
}

#[test]
fn unrelated_timeout_does_not_claim_network_diagnosis() {
    assert!(run_diagnostic(Some("worker timed out"), None, "").is_none());
}

#[tokio::test]
async fn surface_management_is_owner_scoped_and_grantable() {
    let _guard = TEST_DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let (owner_id, _bear_id, _bear_slug) = seed_member(&pool).await;
    let (other_id, _other_bear, _other_bear_slug) = seed_member(&pool).await;
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
    let (user_id, bear_id, bear_slug) = seed_member(&pool).await;
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
    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/new"),
        job_body.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Assign the bear from the surface page, then the same form succeeds and
    // binds the canonical surface id.
    let response = post_form(
        &app,
        &cookie,
        &format!("/work/surfaces/{surface_id}/bears/assign"),
        format!("bear_id={bear_id}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let response = post_form(
        &app,
        &cookie,
        &format!("/bear/{bear_slug}/jobs/new"),
        job_body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let bound_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT work_surface_id FROM bear_jobs
         WHERE bear_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("job row");
    assert_eq!(bound_id, Some(surface_id));
}
