use den_core::BearProfile;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

use crate::{
    docket_task_status_from_task_list_item_status, task_list_projection_from_docket_job,
    DocketCommitPolicy, DocketConversationObjectiveRequest, DocketCriterionKind,
    DocketCriterionStateUpdate, DocketEffortHint, DocketExecutionLookup, DocketJobCreate,
    DocketJobCriterionInput, DocketJobExecuteRequest, DocketJobStatus, DocketService,
    DocketTaskCreate, DocketTaskDefinitionPatch, DocketTaskDifficulty, DocketTaskInput,
    DocketTaskKind, DocketTaskListFilter, DocketTaskRunStateUpdate, DocketTaskScope,
    DocketTaskStatus, DocketTaskUpdate, PgDocketService, TaskDispatcher, TaskListCheckoutRequest,
    TaskListCheckoutSource, TaskListSyncRequest, TaskListVisibility,
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
        r"
        INSERT INTO users (email, username, display_name)
        VALUES ($1, $2, $3)
        RETURNING id
        ",
    )
    .bind(email)
    .bind(username)
    .bind("Docket Test")
    .fetch_one(pool)
    .await
    .expect("seed user");

    let slug = format!("docket-{label}-{}", &suffix[..12]);
    let (bear_id,): (Uuid,) = sqlx::query_as(
        r"
        INSERT INTO bears (slug, name, description)
        VALUES ($1, $2, $3)
        RETURNING id
        ",
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
        work_surface_id: None,
        commit_policy: Some(DocketCommitPolicy::ProposeOnly),
        work_branch: None,
        status: DocketJobStatus::Ready,
        visibility: TaskListVisibility::SameUser,
        source_conversation_id: None,
        objective_kind: None,
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
                completion_criteria: vec!["First task is actually done".to_string()],
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
                completion_criteria: vec!["Second task is actually done".to_string()],
                difficulty: Some(DocketTaskDifficulty::Trivial),
                effort_hint: Some(DocketEffortHint::Low),
                assigned_to_role: Some(BearProfile::Pair),
            },
        ],
    }
}

#[tokio::test]
async fn conversation_objective_checkout_projects_active_subtree_after_reconnect() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket integration test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "conversation-objective").await;
    let service = PgDocketService::from_pool(&pool);

    let first = service
        .get_or_create_conversation_objective(DocketConversationObjectiveRequest {
            bear_id,
            created_by_user_id: user_id,
            created_by_role: "pair".to_string(),
            source_conversation_id: "conversation-1".to_string(),
        })
        .await
        .expect("create conversation objective");
    let second = service
        .get_or_create_conversation_objective(DocketConversationObjectiveRequest {
            bear_id,
            created_by_user_id: user_id,
            created_by_role: "pair".to_string(),
            source_conversation_id: "conversation-1".to_string(),
        })
        .await
        .expect("reuse conversation objective");

    assert_eq!(first.job.id, second.job.id);
    assert_eq!(
        first.job.source_conversation_id.as_deref(),
        Some("conversation-1")
    );
    assert_eq!(
        first.job.objective_kind.as_deref(),
        Some("conversation_task_list")
    );

    let (count,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM bear_jobs
        WHERE bear_id = $1
          AND source_conversation_id = 'conversation-1'
          AND objective_kind = 'conversation_task_list'
          AND status NOT IN ('completed', 'cancelled')
        "#,
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("count conversation objectives");
    assert_eq!(count, 1);

    let parent = service
        .create_task(DocketTaskCreate {
            bear_id,
            job_id: Some(first.job.id),
            session_anchor_id: None,
            parent_task_id: None,
            sibling_order: 0,
            kind: DocketTaskKind::Execution,
            scope: DocketTaskScope::Template,
            title: "Parent task".to_string(),
            body: "Own the active subtree".to_string(),
            completion_criteria: vec!["Subtree is projected".to_string()],
            difficulty: Some(DocketTaskDifficulty::Trivial),
            effort_hint: Some(DocketEffortHint::Low),
            assigned_to_role: Some(BearProfile::Pair),
            created_by_role: "pair".to_string(),
            created_by_user_id: Some(user_id),
            created_by_agent_id: None,
            created_in_run_id: first.job.current_run_id,
        })
        .await
        .expect("create parent task");
    let child = service
        .create_task(DocketTaskCreate {
            bear_id,
            job_id: Some(first.job.id),
            session_anchor_id: None,
            parent_task_id: Some(parent.id),
            sibling_order: 0,
            kind: DocketTaskKind::Execution,
            scope: DocketTaskScope::Template,
            title: "Child task".to_string(),
            body: "Stay active across reconnect".to_string(),
            completion_criteria: vec!["Child is current".to_string()],
            difficulty: Some(DocketTaskDifficulty::Trivial),
            effort_hint: Some(DocketEffortHint::Low),
            assigned_to_role: Some(BearProfile::Pair),
            created_by_role: "pair".to_string(),
            created_by_user_id: Some(user_id),
            created_by_agent_id: None,
            created_in_run_id: first.job.current_run_id,
        })
        .await
        .expect("create child task");
    service
        .update_task(DocketTaskUpdate {
            bear_id,
            job_id: None,
            task_id: child.id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            definition: DocketTaskDefinitionPatch::default(),
            run_state: Some(DocketTaskRunStateUpdate {
                run_id: first
                    .job
                    .current_run_id
                    .expect("conversation objective run"),
                status: DocketTaskStatus::InProgress,
                result_refs: None,
                result_summary: None,
            }),
        })
        .await
        .expect("mark child active");

    let projected = service
        .checkout_task_list(
            bear_id,
            BearProfile::Pair,
            user_id,
            TaskListCheckoutRequest {
                source: TaskListCheckoutSource::ConversationObjective {
                    request: DocketConversationObjectiveRequest {
                        bear_id,
                        created_by_user_id: user_id,
                        created_by_role: "pair".to_string(),
                        source_conversation_id: "conversation-1".to_string(),
                    },
                    active_subtree: true,
                },
            },
        )
        .await
        .expect("checkout conversation objective")
        .expect("conversation objective projection");
    assert_eq!(projected.id, first.job.id);
    assert_eq!(projected.items.len(), 1);
    assert_eq!(projected.items[0].id, child.id.to_string());
    assert_eq!(
        projected.current_item.as_ref().map(|item| item.id.as_str()),
        Some(child.id.to_string().as_str())
    );
}

#[tokio::test]
async fn creates_session_anchored_task_without_job() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket integration test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "session-task").await;
    let service = PgDocketService::from_pool(&pool);
    let (session_anchor_id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO client_sessions (
            user_id, bear_id, bear_slug, client_session_id, runtime_session_id, conversation_id, client
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(bear_id)
    .bind("docket-session-task")
    .bind("session-task-client")
    .bind("session-task-runtime")
    .bind("session-task-conversation")
    .bind("test")
    .fetch_one(&pool)
    .await
    .expect("seed client session");

    let task = service
        .create_task(DocketTaskCreate {
            bear_id,
            job_id: None,
            session_anchor_id: Some(session_anchor_id),
            parent_task_id: None,
            sibling_order: 0,
            kind: DocketTaskKind::Investigation,
            scope: DocketTaskScope::Run,
            title: "Session anchored task".to_string(),
            body: "Confirm jobless task creation works".to_string(),
            completion_criteria: vec!["Task row is inserted".to_string()],
            difficulty: Some(DocketTaskDifficulty::Trivial),
            effort_hint: Some(DocketEffortHint::Low),
            assigned_to_role: Some(BearProfile::Pair),
            created_by_role: "pair".to_string(),
            created_by_user_id: Some(user_id),
            created_by_agent_id: None,
            created_in_run_id: None,
        })
        .await
        .expect("create session-anchored task");

    assert_eq!(task.job_id, None);
    assert_eq!(task.session_anchor_id, Some(session_anchor_id));
    assert_eq!(task.body, "Confirm jobless task creation works");
    assert_eq!(task.completion_criteria.0, vec!["Task row is inserted"]);
}

#[tokio::test]
async fn lists_session_anchored_task_with_latest_run_state() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket integration test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "session-task-state").await;
    let service = PgDocketService::from_pool(&pool);
    let (session_anchor_id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO client_sessions (
            user_id, bear_id, bear_slug, client_session_id, runtime_session_id, conversation_id, client
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(bear_id)
    .bind("docket-session-task-state")
    .bind("session-task-state-client")
    .bind("session-task-state-runtime")
    .bind("session-task-state-conversation")
    .bind("test")
    .fetch_one(&pool)
    .await
    .expect("seed client session");
    let job = service
        .create_job(DocketJobCreate {
            tasks: vec![],
            ..two_task_job(user_id, bear_id)
        })
        .await
        .expect("create run source job");
    let run_id = job.job.current_run_id.expect("current run");
    let task = service
        .create_task(DocketTaskCreate {
            bear_id,
            job_id: None,
            session_anchor_id: Some(session_anchor_id),
            parent_task_id: None,
            sibling_order: 0,
            kind: DocketTaskKind::Execution,
            scope: DocketTaskScope::Run,
            title: "Session task with state".to_string(),
            body: "Confirm session task status projection works".to_string(),
            completion_criteria: vec!["Task status is projected".to_string()],
            difficulty: Some(DocketTaskDifficulty::Trivial),
            effort_hint: Some(DocketEffortHint::Low),
            assigned_to_role: Some(BearProfile::Pair),
            created_by_role: "pair".to_string(),
            created_by_user_id: Some(user_id),
            created_by_agent_id: None,
            created_in_run_id: None,
        })
        .await
        .expect("create session task");
    service
        .update_task(DocketTaskUpdate {
            bear_id,
            job_id: None,
            task_id: task.id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            definition: DocketTaskDefinitionPatch::default(),
            run_state: Some(DocketTaskRunStateUpdate {
                run_id,
                status: DocketTaskStatus::Done,
                result_refs: None,
                result_summary: Some("Verified status projection".to_string()),
            }),
        })
        .await
        .expect("mark session task done");

    let tasks = service
        .list_tasks(
            bear_id,
            DocketTaskListFilter {
                session_anchor_id: Some(session_anchor_id),
                ..DocketTaskListFilter::default()
            },
        )
        .await
        .expect("list session tasks");

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task.id, task.id);
    assert_eq!(
        tasks[0]
            .run_state
            .as_ref()
            .map(|state| state.status.as_str()),
        Some("done")
    );
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
            source_client_session_id: Some("pair-integration-session".to_string()),
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
                source_client_session_id: Some("pair-integration-session".to_string()),
            },
        )
        .await
        .expect("lookup active execution")
        .expect("active execution binding");
    assert_eq!(active_execution.job_id, created.job.id);
    assert_eq!(active_execution.run_id, run_id);
    assert_eq!(active_execution.task_id, Some(first_task_id));
    assert_eq!(active_execution.state, "active");

    let (focus_event_count,): (i64,) = sqlx::query_as(
        r#"
        SELECT count(*)
        FROM bear_job_events
        WHERE job_id = $1
          AND run_id = $2
          AND task_id = $3
          AND event_type = 'focus_selected'
          AND payload->>'session_id' = 'pair-integration-session'
          AND payload->>'source_client_session_id' = 'pair-integration-session'
          AND payload->>'state' = 'active'
        "#,
    )
    .bind(created.job.id)
    .bind(run_id)
    .bind(first_task_id)
    .fetch_one(&pool)
    .await
    .expect("query focus event");
    assert_eq!(focus_event_count, 1);

    let (task_definition_count,): (i64,) = sqlx::query_as(
        r#"
        SELECT count(*)
        FROM bear_task_events
        WHERE task_id = $1
          AND run_id = $2
          AND event_type = 'created'
          AND payload->'definition'->>'title' = 'First task'
          AND payload->'definition'->>'body' = 'Do first task'
          AND payload->'definition'->'completion_criteria'->>0 = 'First task is actually done'
        "#,
    )
    .bind(first_task_id)
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("query task definition event");
    assert_eq!(task_definition_count, 1);

    let missing_summary = service
        .update_task(DocketTaskUpdate {
            bear_id,
            job_id: None,
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
            job_id: None,
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
            session_id: Some("pair-integration-session".to_string()),
            source_conversation_id: None,
            source_client_session_id: Some("pair-integration-session".to_string()),
        })
        .await
        .expect("execute second");
    assert_eq!(second.selected_task_id, Some(second_task_id));

    service
        .update_task(DocketTaskUpdate {
            bear_id,
            job_id: None,
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
            session_id: Some("pair-integration-session".to_string()),
            source_conversation_id: None,
            source_client_session_id: Some("pair-integration-session".to_string()),
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
            session_id: Some("pair-integration-session".to_string()),
            source_conversation_id: None,
            source_client_session_id: Some("pair-integration-session".to_string()),
        })
        .await
        .expect("complete job");
    assert!(completed.completed);
    assert_eq!(completed.job.job.status, "completed");
    assert_eq!(
        completed.job.current_run.as_ref().unwrap().state,
        "completed"
    );
    let stale_execution = service
        .get_active_execution_session(
            bear_id,
            BearProfile::Pair,
            DocketExecutionLookup {
                session_id: None,
                source_conversation_id: None,
                source_client_session_id: Some("pair-integration-session".to_string()),
            },
        )
        .await
        .expect("lookup stale execution");
    assert!(stale_execution.is_none());
}

#[tokio::test]
async fn docket_execution_focus_prefers_conversation_over_client_session() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket conversation focus test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "conversation-focus").await;
    let service = PgDocketService::from_pool(&pool);
    let created = service
        .create_job(two_task_job(user_id, bear_id))
        .await
        .expect("create job");
    let run_id = created.job.current_run_id.expect("current run");
    let first_task_id = created.tasks[0].id;

    let selected = service
        .execute_job(DocketJobExecuteRequest {
            bear_id,
            job_id: created.job.id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            session_id: Some("adapter-session-1".to_string()),
            source_conversation_id: Some("conversation-1".to_string()),
            source_client_session_id: Some("client-session-1".to_string()),
        })
        .await
        .expect("execute first");
    assert_eq!(selected.selected_task_id, Some(first_task_id));

    let active_execution = service
        .get_active_execution_session(
            bear_id,
            BearProfile::Pair,
            DocketExecutionLookup {
                session_id: None,
                source_conversation_id: Some("conversation-1".to_string()),
                source_client_session_id: Some("client-session-2".to_string()),
            },
        )
        .await
        .expect("lookup active execution")
        .expect("conversation-bound active execution");
    assert_eq!(active_execution.session_id, "conversation:conversation-1");
    assert_eq!(active_execution.job_id, created.job.id);
    assert_eq!(active_execution.run_id, run_id);
    assert_eq!(active_execution.task_id, Some(first_task_id));

    service
        .execute_job(DocketJobExecuteRequest {
            bear_id,
            job_id: created.job.id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            session_id: Some("adapter-session-2".to_string()),
            source_conversation_id: Some("conversation-1".to_string()),
            source_client_session_id: Some("client-session-2".to_string()),
        })
        .await
        .expect("execute after reconnect");

    let (active_rows,): (i64,) = sqlx::query_as(
        r#"
        SELECT count(*)
        FROM docket_execution_sessions
        WHERE bear_id = $1
          AND owner_profile = 'pair'
          AND source_conversation_id = 'conversation-1'
          AND state IN ('active', 'blocked', 'completing', 'paused')
        "#,
    )
    .bind(bear_id)
    .fetch_one(&pool)
    .await
    .expect("count active conversation rows");
    assert_eq!(active_rows, 1);
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
    task_list.items[0].status = crate::TaskListItemStatus::Completed;
    task_list.items[0].summary = Some(created.tasks[0].body.clone());
    assert_eq!(
        docket_task_status_from_task_list_item_status(task_list.items[0].status).as_str(),
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

#[tokio::test]
async fn docket_dispatcher_finds_starts_and_records_work_task_outcomes() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket dispatcher test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "dispatcher").await;
    let service = PgDocketService::from_pool(&pool);
    let mut create = two_task_job(user_id, bear_id);
    create.tasks[0].assigned_to_role = Some(BearProfile::Work);
    create.tasks[1].parent_client_key = Some("first".to_string());
    create.tasks[1].assigned_to_role = Some(BearProfile::Work);

    let created = service.create_job(create).await.expect("create job");
    let run_id = created.job.current_run_id.expect("current run");
    let parent_task_id = created.tasks[0].id;
    let child_task_id = created.tasks[1].id;
    assert_eq!(created.tasks[1].parent_task_id, Some(parent_task_id));

    let runnable = service
        .runnable_work_tasks(bear_id, 10)
        .await
        .expect("runnable work tasks");
    assert_eq!(runnable.len(), 2);
    assert!(runnable.iter().any(|task| task.task.id == parent_task_id));
    assert!(runnable.iter().any(|task| task.task.id == child_task_id));

    let started = service
        .mark_task_started(
            bear_id,
            parent_task_id,
            run_id,
            Some("dispatcher-test".to_string()),
        )
        .await
        .expect("mark started");
    assert_eq!(started.run_state.as_ref().unwrap().status, "in_progress");

    let done = service
        .record_task_success(
            bear_id,
            parent_task_id,
            run_id,
            "work task completed".to_string(),
            Some(serde_json::json!({"artifact":"dispatcher-test"})),
            Some("dispatcher-test".to_string()),
        )
        .await
        .expect("record success");
    assert_eq!(done.run_state.as_ref().unwrap().status, "done");
    assert_eq!(
        done.run_state.as_ref().unwrap().result_summary.as_deref(),
        Some("work task completed")
    );

    let blocked = service
        .record_task_blocked(
            bear_id,
            child_task_id,
            run_id,
            "waiting for sandbox capability".to_string(),
            None,
            Some("dispatcher-test".to_string()),
        )
        .await
        .expect("record blocked");
    assert_eq!(blocked.run_state.as_ref().unwrap().status, "blocked");
    assert_eq!(
        blocked
            .run_state
            .as_ref()
            .unwrap()
            .result_summary
            .as_deref(),
        Some("waiting for sandbox capability")
    );
}
