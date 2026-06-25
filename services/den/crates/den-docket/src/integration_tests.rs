use den_core::BearProfile;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

use crate::{
    docket_task_status_from_work_plan_item_status, task_list_projection_from_docket_job,
    DocketCommitPolicy, DocketCriterionKind, DocketCriterionStateUpdate, DocketEffortHint,
    DocketExecutionLookup, DocketJobCreate, DocketJobCriterionInput, DocketJobExecuteRequest,
    DocketJobStatus, DocketService, DocketTaskDefinitionPatch, DocketTaskDifficulty,
    DocketTaskInput, DocketTaskKind, DocketTaskRunStateUpdate, DocketTaskScope, DocketTaskStatus,
    DocketTaskUpdate, PgDocketService, TaskListSyncRequest, WorkPlanVisibility,
};

async fn test_pool() -> Option<PgPool> {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .ok()?;
    sqlx::migrate!("../../migrations").run(&pool).await.ok()?;
    Some(pool)
}

async fn seed_user_and_bear(pool: &PgPool, label: &str) -> (i32, Uuid) {
    let suffix = Uuid::new_v4().simple().to_string();
    let username = format!("u{}", &suffix[..20]);
    let email = format!("{label}-{suffix}@example.test");
    let (user_id,): (i32,) = sqlx::query_as(
        r#"
        INSERT INTO users (email, username, display_name)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(email)
    .bind(username)
    .bind("Docket Test")
    .fetch_one(pool)
    .await
    .expect("seed user");

    let slug = format!("docket-{label}-{}", &suffix[..12]);
    let (bear_id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO bears (slug, name, description)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(slug)
    .bind("Docket Test Bear")
    .bind("integration test bear")
    .fetch_one(pool)
    .await
    .expect("seed bear");

    (user_id, bear_id)
}

fn two_task_job(user_id: i32, bear_id: Uuid) -> DocketJobCreate {
    DocketJobCreate {
        bear_id,
        created_by_user_id: user_id,
        created_by_role: "pair".to_string(),
        goal: "Docket integration lifecycle".to_string(),
        work_surface_ref: None,
        commit_policy: Some(DocketCommitPolicy::ProposeOnly),
        status: DocketJobStatus::Ready,
        visibility: WorkPlanVisibility::SameUser,
        criteria: vec![DocketJobCriterionInput {
            kind: DocketCriterionKind::Narrative,
            description: "Both tasks are done".to_string(),
            spec: None,
            sibling_order: 0,
        }],
        tasks: vec![
            DocketTaskInput {
                client_key: Some("first".to_string()),
                parent_client_key: None,
                parent_task_id: None,
                sibling_order: 0,
                kind: DocketTaskKind::Execution,
                scope: DocketTaskScope::Template,
                title: "First task".to_string(),
                body: "Do first task".to_string(),
                difficulty: Some(DocketTaskDifficulty::Trivial),
                effort_hint: Some(DocketEffortHint::Low),
                assigned_to_role: Some(BearProfile::Pair),
            },
            DocketTaskInput {
                client_key: Some("second".to_string()),
                parent_client_key: None,
                parent_task_id: None,
                sibling_order: 1,
                kind: DocketTaskKind::Execution,
                scope: DocketTaskScope::Template,
                title: "Second task".to_string(),
                body: "Do second task".to_string(),
                difficulty: Some(DocketTaskDifficulty::Trivial),
                effort_hint: Some(DocketEffortHint::Low),
                assigned_to_role: Some(BearProfile::Pair),
            },
        ],
    }
}

#[tokio::test]
async fn docket_pair_lifecycle_completes_after_tasks_and_criteria() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket integration test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "lifecycle").await;
    let service = PgDocketService::from_pool(&pool);

    let created = service
        .create_job(two_task_job(user_id, bear_id))
        .await
        .expect("create job");
    let run_id = created.job.current_run_id.expect("current run");
    let criterion_id = created.criteria[0].id;
    let first_task_id = created.tasks[0].id;
    let second_task_id = created.tasks[1].id;

    let first = service
        .execute_job(DocketJobExecuteRequest {
            bear_id,
            job_id: created.job.id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            session_id: Some("pair-integration-session".to_string()),
            source_conversation_id: None,
            source_acp_session_id: Some("pair-integration-session".to_string()),
        })
        .await
        .expect("execute first");
    assert_eq!(first.selected_task_id, Some(first_task_id));
    assert_eq!(first.job.job.status, "running");
    let active_execution = service
        .get_active_execution_session(
            bear_id,
            BearProfile::Pair,
            DocketExecutionLookup {
                session_id: None,
                source_conversation_id: None,
                source_acp_session_id: Some("pair-integration-session".to_string()),
            },
        )
        .await
        .expect("lookup active execution")
        .expect("active execution binding");
    assert_eq!(active_execution.job_id, created.job.id);
    assert_eq!(active_execution.run_id, run_id);
    assert_eq!(active_execution.task_id, Some(first_task_id));
    assert_eq!(active_execution.state, "active");

    let missing_summary = service
        .update_task(DocketTaskUpdate {
            bear_id,
            task_id: first_task_id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            definition: DocketTaskDefinitionPatch::default(),
            run_state: Some(DocketTaskRunStateUpdate {
                run_id,
                status: DocketTaskStatus::Done,
                result_refs: None,
                result_summary: None,
            }),
        })
        .await;
    assert!(missing_summary.is_err());

    service
        .update_task(DocketTaskUpdate {
            bear_id,
            task_id: first_task_id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            definition: DocketTaskDefinitionPatch::default(),
            run_state: Some(DocketTaskRunStateUpdate {
                run_id,
                status: DocketTaskStatus::Done,
                result_refs: None,
                result_summary: Some("First task actually completed".to_string()),
            }),
        })
        .await
        .expect("complete first");

    let second = service
        .execute_job(DocketJobExecuteRequest {
            bear_id,
            job_id: created.job.id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            session_id: None,
            source_conversation_id: None,
            source_acp_session_id: None,
        })
        .await
        .expect("execute second");
    assert_eq!(second.selected_task_id, Some(second_task_id));

    service
        .update_task(DocketTaskUpdate {
            bear_id,
            task_id: second_task_id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            definition: DocketTaskDefinitionPatch::default(),
            run_state: Some(DocketTaskRunStateUpdate {
                run_id,
                status: DocketTaskStatus::Done,
                result_refs: None,
                result_summary: Some("Second task actually completed".to_string()),
            }),
        })
        .await
        .expect("complete second");

    let blocked = service
        .execute_job(DocketJobExecuteRequest {
            bear_id,
            job_id: created.job.id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            session_id: None,
            source_conversation_id: None,
            source_acp_session_id: None,
        })
        .await
        .expect("blocked before criteria");
    assert!(blocked.blocked);
    assert_eq!(blocked.job.job.status, "blocked");

    let evaluated = service
        .evaluate_criterion(DocketCriterionStateUpdate {
            bear_id,
            job_id: created.job.id,
            run_id,
            criterion_id,
            status: crate::DocketCriterionStatus::Met,
            evidence: Some(serde_json::json!({"summary":"Both tasks are done"})),
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
        })
        .await
        .expect("evaluate criterion");
    assert_eq!(evaluated.criteria_states[0].status, "met");

    let completed = service
        .execute_job(DocketJobExecuteRequest {
            bear_id,
            job_id: created.job.id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            session_id: None,
            source_conversation_id: None,
            source_acp_session_id: None,
        })
        .await
        .expect("complete job");
    assert!(completed.completed);
    assert_eq!(completed.job.job.status, "completed");
    assert_eq!(
        completed.job.current_run.as_ref().unwrap().state,
        "completed"
    );
}

#[tokio::test]
async fn docket_task_list_sync_rejects_completed_item_without_evidence() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket sync test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "sync").await;
    let service = PgDocketService::from_pool(&pool);
    let created = service
        .create_job(two_task_job(user_id, bear_id))
        .await
        .expect("create job");
    let mut task_list = task_list_projection_from_docket_job(&created, None);
    task_list.owner_profile = "pair".to_string();
    task_list.items[0].status = crate::WorkPlanItemStatus::Completed;
    task_list.items[0].summary = Some(created.tasks[0].body.clone());
    assert_eq!(
        docket_task_status_from_work_plan_item_status(task_list.items[0].status).as_str(),
        "done"
    );

    let outcome = service
        .sync_task_list(TaskListSyncRequest { task_list })
        .await
        .expect("sync outcome");
    assert!(!outcome.applied);
    assert!(!outcome.conflicts.is_empty());
    assert!(outcome.conflicts[0].contains("completion summary"));
}
