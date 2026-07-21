//! Minimal dispatch seam for job-level work execution.
//!
//! Docket owns job/task state, but it must not execute task bodies itself
//! (ADR-0034 execution invariant). Runtime code can use this trait to poll for
//! runnable work tasks and record outcomes after a Bear runtime executes them.

use uuid::Uuid;

use den_core::{BearProfile, DenError};

use crate::db;
use crate::model::{
    DocketTaskDefinitionPatch, DocketTaskListFilter, DocketTaskProjection,
    DocketTaskRunStateUpdate, DocketTaskStatus, DocketTaskUpdate,
};
use crate::service::PgDocketService;

#[allow(async_fn_in_trait)]
pub trait TaskDispatcher: Send + Sync {
    async fn runnable_work_tasks(
        &self,
        bear_id: Uuid,
        limit: i64,
    ) -> Result<Vec<DocketTaskProjection>, DenError>;

    async fn mark_task_started(
        &self,
        bear_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        actor_agent_id: Option<String>,
    ) -> Result<DocketTaskProjection, DenError>;

    async fn record_task_success(
        &self,
        bear_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        result_summary: String,
        result_refs: Option<serde_json::Value>,
        actor_agent_id: Option<String>,
    ) -> Result<DocketTaskProjection, DenError>;

    async fn record_task_blocked(
        &self,
        bear_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        blocked_reason: String,
        result_refs: Option<serde_json::Value>,
        actor_agent_id: Option<String>,
    ) -> Result<DocketTaskProjection, DenError>;
}

impl TaskDispatcher for PgDocketService {
    async fn runnable_work_tasks(
        &self,
        bear_id: Uuid,
        limit: i64,
    ) -> Result<Vec<DocketTaskProjection>, DenError> {
        let scan_limit = if limit <= 0 { 100 } else { limit.min(500) };
        // ponytail: simple DB polling over the task tree is enough for the first
        // Docket slice; upgrade to queue/lease-based dispatch when multiple
        // workers compete or the task table is large.
        let tasks = db::list_tasks(
            &self.pool,
            bear_id,
            DocketTaskListFilter {
                include_descendants: true,
                limit: scan_limit,
                ..DocketTaskListFilter::default()
            },
        )
        .await?;

        Ok(tasks
            .into_iter()
            .filter(|projection| {
                projection
                    .run_state
                    .as_ref()
                    .map(|state| state.status.as_str())
                    .unwrap_or("pending")
                    == "pending"
            })
            .collect())
    }

    async fn mark_task_started(
        &self,
        bear_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        actor_agent_id: Option<String>,
    ) -> Result<DocketTaskProjection, DenError> {
        update_run_state(
            self,
            bear_id,
            task_id,
            run_id,
            DocketTaskStatus::InProgress,
            None,
            None,
            actor_agent_id,
        )
        .await
    }

    async fn record_task_success(
        &self,
        bear_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        result_summary: String,
        result_refs: Option<serde_json::Value>,
        actor_agent_id: Option<String>,
    ) -> Result<DocketTaskProjection, DenError> {
        update_run_state(
            self,
            bear_id,
            task_id,
            run_id,
            DocketTaskStatus::Done,
            result_refs,
            Some(result_summary),
            actor_agent_id,
        )
        .await
    }

    async fn record_task_blocked(
        &self,
        bear_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
        blocked_reason: String,
        result_refs: Option<serde_json::Value>,
        actor_agent_id: Option<String>,
    ) -> Result<DocketTaskProjection, DenError> {
        update_run_state(
            self,
            bear_id,
            task_id,
            run_id,
            DocketTaskStatus::Blocked,
            result_refs,
            Some(blocked_reason),
            actor_agent_id,
        )
        .await
    }
}

async fn update_run_state(
    service: &PgDocketService,
    bear_id: Uuid,
    task_id: Uuid,
    run_id: Uuid,
    status: DocketTaskStatus,
    result_refs: Option<serde_json::Value>,
    result_summary: Option<String>,
    actor_agent_id: Option<String>,
) -> Result<DocketTaskProjection, DenError> {
    db::update_task(
        &service.pool,
        DocketTaskUpdate {
            bear_id,
            job_id: None,
            task_id,
            actor_role: BearProfile::Work,
            actor_user_id: None,
            actor_agent_id,
            definition: DocketTaskDefinitionPatch::default(),
            run_state: Some(DocketTaskRunStateUpdate {
                run_id,
                status,
                result_refs,
                result_summary,
            }),
        },
    )
    .await
}
