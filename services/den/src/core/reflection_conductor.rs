use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use std::sync::Arc;

use crate::{
    config::Config,
    core::{
        conversation_events::{
            memory_curate_completed_projection, memory_curate_enqueued_projection,
            memory_curate_failed_projection, memory_curate_started_projection, project_to_conversation,
            ProjectionProvenance, ProjectionSource,
        },
        memory::{
            record_reflection_outcome_complete, record_reflection_outcome_start, MemoryStoreManager,
        },
        memory_curate_executor::{self, MemoryCurateRunOutput},
        reflection_conversations::{
            bind_memory_curate_run_conversation, ensure_memory_curate_conversation,
            touch_memory_curate_conversation,
        },
    },
    errors::CustomError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionRunRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub lane: String,
    pub trigger: String,
    pub status: String,
    pub role_agent_id: Option<String>,
    pub conversation_id: Option<String>,
    pub conversation_key: Option<String>,
    pub conversation_date: Option<Date>,
    pub input_summary: serde_json::Value,
    pub output_summary: serde_json::Value,
    pub error: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct CreateReflectionRun<'a> {
    pub bear_id: Uuid,
    pub lane: &'a str,
    pub trigger: &'a str,
    pub status: &'a str,
    pub role_agent_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub conversation_key: Option<&'a str>,
    pub conversation_date: Option<Date>,
    pub input_summary: serde_json::Value,
    pub output_summary: serde_json::Value,
    pub error: Option<&'a str>,
}

pub async fn create_run(
    pool: &PgPool,
    params: CreateReflectionRun<'_>,
) -> Result<ReflectionRunRow, CustomError> {
    let row = sqlx::query(
        r#"
        INSERT INTO bear_reflection_runs (
            bear_id, lane, trigger, status, role_agent_id,
            conversation_id, conversation_key, conversation_date,
            input_summary, output_summary, error
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id, bear_id, lane, trigger, status, role_agent_id,
                  conversation_id, conversation_key, conversation_date,
                  input_summary, output_summary, error,
                  started_at, completed_at, created_at
        "#,
    )
    .bind(params.bear_id)
    .bind(params.lane)
    .bind(params.trigger)
    .bind(params.status)
    .bind(params.role_agent_id)
    .bind(params.conversation_id)
    .bind(params.conversation_key)
    .bind(params.conversation_date)
    .bind(params.input_summary)
    .bind(params.output_summary)
    .bind(params.error)
    .fetch_one(pool)
    .await?;
    Ok(row_from_sql(row))
}

pub struct ProposalEnqueueParams<'a> {
    pub bear_id: Uuid,
    pub role_agent_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub conversation_key: Option<&'a str>,
    pub conversation_date: Option<Date>,
    pub trigger: &'a str,
    pub proposal_ids: Vec<Uuid>,
}

fn reflection_conductor_provenance(row: &ReflectionRunRow) -> ProjectionProvenance {
    ProjectionProvenance {
        source: ProjectionSource::ReflectionConductor,
        scope_id: format!("bear:{}:lane:{}", row.bear_id, row.lane),
    }
}

fn project_memory_curate_enqueued(pool: &PgPool, row: &ReflectionRunRow, proposal_ids: Vec<Uuid>) {
    project_to_conversation(
        pool,
        row.bear_id,
        None,
        row.conversation_id.as_deref(),
        memory_curate_enqueued_projection(
            reflection_conductor_provenance(row),
            row.id,
            row.lane.clone(),
            row.trigger.clone(),
            row.status.clone(),
            proposal_ids,
            row.conversation_key.clone(),
            row.conversation_date,
            row.created_at,
        ),
    );
}

fn project_memory_curate_started(pool: &PgPool, row: &ReflectionRunRow, proposal_ids: Vec<Uuid>) {
    project_to_conversation(
        pool,
        row.bear_id,
        None,
        row.conversation_id.as_deref(),
        memory_curate_started_projection(
            reflection_conductor_provenance(row),
            row.id,
            row.lane.clone(),
            row.trigger.clone(),
            row.status.clone(),
            proposal_ids,
            row.conversation_key.clone(),
            row.conversation_date,
            row.started_at,
        ),
    );
}

fn project_memory_curate_completed(
    pool: &PgPool,
    row: &ReflectionRunRow,
    proposal_ids: Vec<Uuid>,
) {
    project_to_conversation(
        pool,
        row.bear_id,
        None,
        row.conversation_id.as_deref(),
        memory_curate_completed_projection(
            reflection_conductor_provenance(row),
            row.id,
            row.lane.clone(),
            row.trigger.clone(),
            row.status.clone(),
            proposal_ids,
            row.conversation_key.clone(),
            row.conversation_date,
            row.completed_at,
        ),
    );
}

fn project_memory_curate_failed(pool: &PgPool, row: &ReflectionRunRow, proposal_ids: Vec<Uuid>) {
    project_to_conversation(
        pool,
        row.bear_id,
        None,
        row.conversation_id.as_deref(),
        memory_curate_failed_projection(
            reflection_conductor_provenance(row),
            row.id,
            row.lane.clone(),
            row.trigger.clone(),
            row.status.clone(),
            proposal_ids,
            row.conversation_key.clone(),
            row.conversation_date,
            row.error.clone(),
            row.completed_at,
        ),
    );
}

pub async fn enqueue_memory_curate_for_proposals(
    pool: &PgPool,
    params: ProposalEnqueueParams<'_>,
) -> Result<ReflectionRunRow, CustomError> {
    let proposal_ids = params.proposal_ids;
    let proposal_id_values: Vec<serde_json::Value> = proposal_ids
        .iter()
        .map(|id| serde_json::Value::String(id.to_string()))
        .collect();
    let row = create_run(
        pool,
        CreateReflectionRun {
            bear_id: params.bear_id,
            lane: "memory_curate",
            trigger: params.trigger,
            status: "queued",
            role_agent_id: params.role_agent_id,
            conversation_id: params.conversation_id,
            conversation_key: params.conversation_key,
            conversation_date: params.conversation_date,
            input_summary: serde_json::json!({ "proposal_ids": proposal_id_values }),
            output_summary: serde_json::json!({}),
            error: None,
        },
    )
    .await?;
    project_memory_curate_enqueued(pool, &row, proposal_ids);
    Ok(row)
}

pub async fn list_queued_memory_curate_runs(
    pool: &PgPool,
    bear_id: Uuid,
    limit: i64,
) -> Result<Vec<ReflectionRunRow>, CustomError> {
    let rows = sqlx::query(
        r#"
        SELECT id, bear_id, lane, trigger, status, role_agent_id,
               conversation_id, conversation_key, conversation_date,
               input_summary, output_summary, error,
               started_at, completed_at, created_at
        FROM bear_reflection_runs
        WHERE bear_id = $1
          AND lane = 'memory_curate'
          AND status = 'queued'
        ORDER BY created_at ASC
        LIMIT $2
        "#,
    )
    .bind(bear_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_from_sql).collect())
}

pub async fn claim_next_memory_curate_run(
    pool: &PgPool,
    bear_id: Uuid,
) -> Result<Option<ReflectionRunRow>, CustomError> {
    let row = sqlx::query(
        r#"
        WITH next_run AS (
            SELECT id
            FROM bear_reflection_runs
            WHERE bear_id = $1
              AND lane = 'memory_curate'
              AND status = 'queued'
            ORDER BY created_at ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE bear_reflection_runs runs
        SET status = 'started',
            started_at = COALESCE(started_at, NOW())
        FROM next_run
        WHERE runs.id = next_run.id
        RETURNING runs.id, runs.bear_id, runs.lane, runs.trigger, runs.status,
                  runs.role_agent_id, runs.conversation_id, runs.conversation_key,
                  runs.conversation_date, runs.input_summary, runs.output_summary,
                  runs.error, runs.started_at, runs.completed_at, runs.created_at
        "#,
    )
    .bind(bear_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut run = row_from_sql(row);
    if let Some(conversation_date) = run.conversation_date {
        let reflection_conversation = ensure_memory_curate_conversation(
            pool,
            run.bear_id,
            run.role_agent_id.as_deref(),
            conversation_date,
        )
        .await?;
        if let Some(conversation_id) = reflection_conversation.conversation_id.as_deref() {
            bind_memory_curate_run_conversation(pool, run.bear_id, run.id, conversation_id)
                .await?;
            run.conversation_id = Some(conversation_id.to_string());
        }
        let _ = touch_memory_curate_conversation(pool, run.bear_id, conversation_date).await;
    }
    project_memory_curate_started(pool, &run, proposal_ids_from_summary(&run.input_summary));
    Ok(Some(run))
}

pub async fn mark_memory_curate_started(
    pool: &PgPool,
    bear_id: Uuid,
    reflection_run_id: Uuid,
) -> Result<ReflectionRunRow, CustomError> {
    let row = sqlx::query(
        r#"
        UPDATE bear_reflection_runs
        SET status = 'started',
            started_at = COALESCE(started_at, NOW())
        WHERE bear_id = $1 AND id = $2 AND lane = 'memory_curate'
        RETURNING id, bear_id, lane, trigger, status, role_agent_id,
                  conversation_id, conversation_key, conversation_date,
                  input_summary, output_summary, error,
                  started_at, completed_at, created_at
        "#,
    )
    .bind(bear_id)
    .bind(reflection_run_id)
    .fetch_one(pool)
    .await?;
    let run = row_from_sql(row);
    project_memory_curate_started(pool, &run, proposal_ids_from_summary(&run.input_summary));
    Ok(run)
}

pub async fn mark_memory_curate_completed(
    pool: &PgPool,
    bear_id: Uuid,
    reflection_run_id: Uuid,
    output_summary: serde_json::Value,
) -> Result<ReflectionRunRow, CustomError> {
    let row = sqlx::query(
        r#"
        UPDATE bear_reflection_runs
        SET status = 'completed',
            output_summary = $3,
            error = NULL,
            completed_at = NOW()
        WHERE bear_id = $1 AND id = $2 AND lane = 'memory_curate'
        RETURNING id, bear_id, lane, trigger, status, role_agent_id,
                  conversation_id, conversation_key, conversation_date,
                  input_summary, output_summary, error,
                  started_at, completed_at, created_at
        "#,
    )
    .bind(bear_id)
    .bind(reflection_run_id)
    .bind(output_summary)
    .fetch_one(pool)
    .await?;
    let run = row_from_sql(row);
    project_memory_curate_completed(pool, &run, proposal_ids_from_summary(&run.input_summary));
    Ok(run)
}

pub async fn mark_memory_curate_failed(
    pool: &PgPool,
    bear_id: Uuid,
    reflection_run_id: Uuid,
    error: &str,
) -> Result<ReflectionRunRow, CustomError> {
    let row = sqlx::query(
        r#"
        UPDATE bear_reflection_runs
        SET status = 'failed',
            error = $3,
            completed_at = NOW()
        WHERE bear_id = $1 AND id = $2 AND lane = 'memory_curate'
        RETURNING id, bear_id, lane, trigger, status, role_agent_id,
                  conversation_id, conversation_key, conversation_date,
                  input_summary, output_summary, error,
                  started_at, completed_at, created_at
        "#,
    )
    .bind(bear_id)
    .bind(reflection_run_id)
    .bind(error)
    .fetch_one(pool)
    .await?;
    let run = row_from_sql(row);
    project_memory_curate_failed(pool, &run, proposal_ids_from_summary(&run.input_summary));
    Ok(run)
}

pub async fn run_next_memory_curate_once(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
) -> Result<Option<ReflectionRunRow>, CustomError> {
    let Some(run) = claim_next_memory_curate_run(pool, bear_id).await? else {
        return Ok(None);
    };

    if config.uses_native_agent_runtime() {
        let input_summary = run.input_summary.to_string();
        let _ = record_reflection_outcome_start(
            stores,
            bear_id,
            &run.id.to_string(),
            &run.lane,
            &run.trigger,
            Some(input_summary.as_str()),
        )
        .await;
    }

    let proposal_ids = proposal_ids_from_summary(&run.input_summary);
    let output = match execute_memory_curate_run(
        pool,
        config,
        stores,
        run.id,
        run.bear_id,
        run.trigger.as_str(),
        &proposal_ids,
    )
    .await
    {
        Ok(output) => output,
        Err(error) => {
            if config.uses_native_agent_runtime() {
                let _ = record_reflection_outcome_complete(
                    stores,
                    bear_id,
                    &run.id.to_string(),
                    "failed",
                    Some(error.to_string().as_str()),
                    &proposal_ids
                        .iter()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>(),
                )
                .await;
            }
            let failed_run =
                mark_memory_curate_failed(pool, run.bear_id, run.id, &error.to_string()).await?;
            return Ok(Some(failed_run));
        }
    };

    if config.uses_native_agent_runtime() {
        let summary = memory_curate_output_summary(&output).to_string();
        let _ = record_reflection_outcome_complete(
            stores,
            bear_id,
            &run.id.to_string(),
            "completed",
            Some(summary.as_str()),
            &proposal_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
        )
        .await;
    }

    let completed_run = mark_memory_curate_completed(
        pool,
        run.bear_id,
        run.id,
        memory_curate_output_summary(&output),
    )
    .await?;
    Ok(Some(completed_run))
}

async fn execute_memory_curate_run(
    pool: &PgPool,
    config: &Config,
    _stores: &MemoryStoreManager,
    reflection_run_id: Uuid,
    bear_id: Uuid,
    trigger: &str,
    proposal_ids: &[Uuid],
) -> Result<MemoryCurateRunOutput, CustomError> {
    let output = memory_curate_executor::execute_memory_curate_proposals(
        pool,
        config,
        bear_id,
        Some(trigger),
        proposal_ids,
    )
    .await?;
    for outcome in &output.outcomes {
        record_memory_curate_run_item(
            pool,
            reflection_run_id,
            outcome.proposal_id,
            &outcome.status,
        )
        .await?;
    }
    Ok(output)
}

async fn record_memory_curate_run_item(
    pool: &PgPool,
    run_id: Uuid,
    proposal_id: Uuid,
    status: &str,
) -> Result<(), CustomError> {
    sqlx::query(
        r#"
        INSERT INTO bear_reflection_run_items (run_id, item_kind, item_id, status)
        VALUES ($1, 'memory_proposal', $2, $3)
        "#,
    )
    .bind(run_id)
    .bind(proposal_id.to_string())
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

fn memory_curate_output_summary(output: &MemoryCurateRunOutput) -> serde_json::Value {
    serde_json::json!({
        "resolved_proposal_ids": output.resolved_proposal_ids,
        "resolution_status": output.resolution_status,
        "status_counts": output.status_counts,
        "outcomes": output.outcomes,
        "briefing": output.briefing,
    })
}

pub async fn run_memory_curate_worker_loop(
    pool: PgPool,
    config: Arc<Config>,
    worker_token: tokio_util::sync::CancellationToken,
    poll_interval: std::time::Duration,
) -> Result<(), CustomError> {
    loop {
        tokio::select! {
            _ = worker_token.cancelled() => {
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {}
        }

        let bear_ids = list_bears_with_queued_memory_curate_runs(&pool).await?;
        let stores = MemoryStoreManager::new(config.as_ref());
        for bear_id in bear_ids {
            if worker_token.is_cancelled() {
                break;
            }
            if let Some(run) =
                run_next_memory_curate_once(&pool, config.as_ref(), &stores, bear_id).await?
            {
                tracing::info!(
                    bear_id = %bear_id,
                    reflection_run_id = %run.id,
                    status = %run.status,
                    "memory_curate worker processed queued run"
                );
            }
        }
    }
    Ok(())
}

async fn list_bears_with_queued_memory_curate_runs(pool: &PgPool) -> Result<Vec<Uuid>, CustomError> {
    let rows = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT DISTINCT bear_id
        FROM bear_reflection_runs
        WHERE lane = 'memory_curate'
          AND status = 'queued'
        ORDER BY bear_id
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn proposal_ids_from_summary(summary: &serde_json::Value) -> Vec<Uuid> {
    summary
        .get("proposal_ids")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .filter_map(|value| Uuid::parse_str(value).ok())
        .collect()
}

fn row_from_sql(row: sqlx::postgres::PgRow) -> ReflectionRunRow {
    ReflectionRunRow {
        id: row.get("id"),
        bear_id: row.get("bear_id"),
        lane: row.get("lane"),
        trigger: row.get("trigger"),
        status: row.get("status"),
        role_agent_id: row.get("role_agent_id"),
        conversation_id: row.get("conversation_id"),
        conversation_key: row.get("conversation_key"),
        conversation_date: row.get("conversation_date"),
        input_summary: row.get("input_summary"),
        output_summary: row.get("output_summary"),
        error: row.get("error"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        created_at: row.get("created_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        core::{bears::BearAgentRole, memory::MemoryStoreManager, memory_proposals},
    };
    use sqlx::postgres::PgPoolOptions;

    async fn test_pool() -> Option<PgPool> {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
        let pool = PgPoolOptions::new().connect(&database_url).await.ok()?;
        sqlx::migrate!("./migrations").run(&pool).await.ok()?;
        Some(pool)
    }

    #[tokio::test]
    async fn memory_curate_claim_next_run_starts_oldest_queued_run() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let first = enqueue_memory_curate_for_proposals(
            &pool,
            ProposalEnqueueParams {
                bear_id,
                role_agent_id: Some("pair-agent"),
                conversation_id: Some("conv-a"),
                conversation_key: Some("memory_curate:test-a"),
                conversation_date: None,
                trigger: "test",
                proposal_ids: vec![Uuid::new_v4()],
            },
        )
        .await
        .expect("enqueue first run");
        let second = enqueue_memory_curate_for_proposals(
            &pool,
            ProposalEnqueueParams {
                bear_id,
                role_agent_id: Some("pair-agent"),
                conversation_id: Some("conv-b"),
                conversation_key: Some("memory_curate:test-b"),
                conversation_date: None,
                trigger: "test",
                proposal_ids: vec![Uuid::new_v4()],
            },
        )
        .await
        .expect("enqueue second run");

        let queued = list_queued_memory_curate_runs(&pool, bear_id, 10)
            .await
            .expect("list queued runs");
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].id, first.id);
        assert_eq!(queued[1].id, second.id);

        let claimed = claim_next_memory_curate_run(&pool, bear_id)
            .await
            .expect("claim queued run")
            .expect("queued run available");
        assert_eq!(claimed.id, first.id);
        assert_eq!(claimed.status, "started");
        assert!(claimed.started_at.is_some());

        let remaining = list_queued_memory_curate_runs(&pool, bear_id, 10)
            .await
            .expect("list remaining queued runs");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, second.id);
    }

    #[tokio::test]
    async fn memory_curate_worker_loop_processes_queued_runs_until_cancelled() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let proposal = memory_proposals::create(
            &pool,
            memory_proposals::CreateMemoryProposal {
                bear_id,
                source_role: BearAgentRole::Pair,
                source_agent_id: Some("pair-agent".to_string()),
                source_paths: vec!["pair/notes/worker.md".to_string()],
                source_refs: serde_json::json!({"conversation_id": "conv-memory-curate-worker-test"}),
                suggested_action: "unspecified",
                target_ref: None,
                title: "Worker memory curate proposal",
                summary: "summary",
                rationale: "rationale",
                proposed_content: None,
                proposed_patch: None,
                refs: serde_json::json!({}),
                sensitivity: "normal",
                requires_human: false,
                project_to_conversation: false,
            },
        )
        .await
        .expect("create proposal");

        enqueue_memory_curate_for_proposals(
            &pool,
            ProposalEnqueueParams {
                bear_id,
                role_agent_id: Some("pair-agent"),
                conversation_id: Some("conv-memory-curate-worker-test"),
                conversation_key: Some("memory_curate:test-worker"),
                conversation_date: None,
                trigger: "pair_reflection",
                proposal_ids: vec![proposal.id],
            },
        )
        .await
        .expect("enqueue run");

        let token = tokio_util::sync::CancellationToken::new();
        let worker_pool = pool.clone();
        let worker_token = token.clone();
        let config = Arc::new(Config::load());
        let handle = tokio::spawn(async move {
            run_memory_curate_worker_loop(
                worker_pool,
                config,
                worker_token,
                std::time::Duration::from_millis(25),
            )
            .await
        });

        let mut saw_completed = false;
        for _ in 0..40 {
            let runs = sqlx::query(
                "SELECT status FROM bear_reflection_runs WHERE bear_id = $1 AND lane = 'memory_curate' ORDER BY created_at DESC LIMIT 1"
            )
            .bind(bear_id)
            .fetch_all(&pool)
            .await
            .expect("query run status");
            if let Some(row) = runs.first() {
                let status: String = row.get("status");
                if status == "completed" {
                    saw_completed = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(saw_completed, "worker did not complete queued run before timeout");

        token.cancel();
        handle.await.expect("worker join").expect("worker result");

        let updated_proposal = memory_proposals::get_for_bear(&pool, bear_id, proposal.id)
            .await
            .expect("reload proposal")
            .expect("proposal exists");
        assert_eq!(updated_proposal.status, "retained_local");
    }

    #[tokio::test]
    async fn memory_curate_runner_completes_run_and_retains_pair_reflection_proposals_locally() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let proposal = memory_proposals::create(
            &pool,
            memory_proposals::CreateMemoryProposal {
                bear_id,
                source_role: BearAgentRole::Pair,
                source_agent_id: Some("pair-agent".to_string()),
                source_paths: vec!["pair/notes/example.md".to_string()],
                source_refs: serde_json::json!({"conversation_id": "conv-memory-curate-test"}),
                suggested_action: "unspecified",
                target_ref: None,
                title: "Test memory curate proposal",
                summary: "summary",
                rationale: "rationale",
                proposed_content: None,
                proposed_patch: None,
                refs: serde_json::json!({}),
                sensitivity: "normal",
                requires_human: false,
                project_to_conversation: false,
            },
        )
        .await
        .expect("create proposal");

        let queued_run = enqueue_memory_curate_for_proposals(
            &pool,
            ProposalEnqueueParams {
                bear_id,
                role_agent_id: Some("pair-agent"),
                conversation_id: Some("conv-memory-curate-test"),
                conversation_key: Some("memory_curate:test-runner"),
                conversation_date: None,
                trigger: "pair_reflection",
                proposal_ids: vec![proposal.id],
            },
        )
        .await
        .expect("enqueue run");

        let config = Config::load();
        let stores = MemoryStoreManager::new(&config);
        let completed_run = run_next_memory_curate_once(&pool, &config, &stores, bear_id)
            .await
            .expect("run queued memory_curate")
            .expect("queued run processed");
        assert_eq!(completed_run.id, queued_run.id);
        assert_eq!(completed_run.status, "completed");
        assert!(completed_run.started_at.is_some());
        assert!(completed_run.completed_at.is_some());
        assert_eq!(
            completed_run.output_summary["resolution_status"],
            serde_json::json!("retained_local")
        );
        assert_eq!(
            completed_run.output_summary["resolved_proposal_ids"],
            serde_json::json!([proposal.id.to_string()])
        );

        let updated_proposal = memory_proposals::get_for_bear(&pool, bear_id, proposal.id)
            .await
            .expect("reload proposal")
            .expect("proposal exists");
        assert_eq!(updated_proposal.status, "retained_local");
        assert_eq!(updated_proposal.reviewer_role.as_deref(), Some("curate"));
        assert_eq!(
            updated_proposal.reviewer_agent_id.as_deref(),
            Some(memory_curate_executor::MEMORY_CURATE_RUNNER_AGENT_ID)
        );

        let queued = list_queued_memory_curate_runs(&pool, bear_id, 10)
            .await
            .expect("list queued after runner");
        assert!(queued.is_empty());
    }
}
