//! `DocketService`: the single public face of the Docket subsystem.
//!
//! Callers (tools, ACP, web) depend on this trait / the concrete
//! `PgDocketService`, never on `docket::db`. This is the seam the crate split
//! promotes to a `den-docket` crate boundary; `TaskDispatcher` (the runtime
//! inversion trait) is added when `den-runtime` is extracted.

use sqlx::PgPool;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

use super::db;
use super::model::{
    task_list_projection_from_docket_job, DocketCriterionStateUpdate, DocketExecutionLookup,
    DocketExecutionSessionRow, DocketJobCreate, DocketJobExecuteOutcome, DocketJobExecuteRequest,
    DocketJobListFilter, DocketJobProjection, DocketJobRow, DocketJobUpdate, DocketTaskCreate,
    DocketTaskListFilter, DocketTaskProjection, DocketTaskRow, DocketTaskUpdate,
    HistoricalTaskListRow, TaskListCheckoutRequest, TaskListCheckoutSource, TaskListHandoffOutcome,
    TaskListHandoffRequest, TaskListListFilter, TaskListLookup, TaskListProjection,
    TaskListSyncOutcome, TaskListSyncRequest, TaskListUpsert,
};

impl PgDocketService {
    // ponytail: compatibility-only methods keep den.work_plan.* callers compiling
    // after bear_work_plans storage retirement; remove with the public tool API
    // migration to den.task_list.* / Docket-backed operations.
    pub async fn list_visible_work_plans(
        &self,
        _bear_id: Uuid,
        _viewer_role: BearProfile,
        _user_id: i32,
        _filter: TaskListListFilter,
    ) -> Result<Vec<HistoricalTaskListRow>, DenError> {
        Ok(Vec::new())
    }

    pub async fn get_visible_work_plan(
        &self,
        _bear_id: Uuid,
        _viewer_role: BearProfile,
        _user_id: i32,
        _lookup: TaskListLookup,
    ) -> Result<Option<HistoricalTaskListRow>, DenError> {
        Ok(None)
    }

    pub async fn upsert_work_plan(
        &self,
        _upsert: TaskListUpsert,
    ) -> Result<HistoricalTaskListRow, DenError> {
        Err(DenError::system(
            "den.work_plan.update no longer writes retired bear_work_plans storage; use Docket task/job tools",
        ))
    }
}

/// Orchestration API for Docket work plans. The only public entry point to the
/// subsystem's persistence; never execute task bodies here (ADR-0034 execution
/// invariant) — Docket schedules, gates, and records.
// Native async fn in trait: workspace-internal, only ever consumed via generic
// bounds / the concrete `PgDocketService` (never `dyn`), so auto-trait (Send)
// bounds flow through monomorphization and async-trait boxing is unnecessary.
#[allow(async_fn_in_trait)]
pub trait DocketService: Send + Sync {
    async fn create_job(&self, create: DocketJobCreate) -> Result<DocketJobProjection, DenError>;

    async fn list_jobs(
        &self,
        bear_id: Uuid,
        filter: DocketJobListFilter,
    ) -> Result<Vec<DocketJobRow>, DenError>;

    async fn get_job(
        &self,
        bear_id: Uuid,
        job_id: Uuid,
    ) -> Result<Option<DocketJobProjection>, DenError>;

    async fn update_job(&self, update: DocketJobUpdate) -> Result<DocketJobProjection, DenError>;

    async fn evaluate_criterion(
        &self,
        update: DocketCriterionStateUpdate,
    ) -> Result<DocketJobProjection, DenError>;

    async fn execute_job(
        &self,
        request: DocketJobExecuteRequest,
    ) -> Result<DocketJobExecuteOutcome, DenError>;

    async fn get_active_execution_session(
        &self,
        bear_id: Uuid,
        owner_profile: BearProfile,
        lookup: DocketExecutionLookup,
    ) -> Result<Option<DocketExecutionSessionRow>, DenError>;

    async fn create_task(&self, create: DocketTaskCreate) -> Result<DocketTaskRow, DenError>;

    async fn list_tasks(
        &self,
        bear_id: Uuid,
        filter: DocketTaskListFilter,
    ) -> Result<Vec<DocketTaskProjection>, DenError>;

    async fn update_task(&self, update: DocketTaskUpdate)
        -> Result<DocketTaskProjection, DenError>;

    async fn checkout_task_list(
        &self,
        bear_id: Uuid,
        viewer_role: BearProfile,
        user_id: i32,
        request: TaskListCheckoutRequest,
    ) -> Result<Option<TaskListProjection>, DenError>;

    async fn sync_task_list(
        &self,
        request: TaskListSyncRequest,
    ) -> Result<TaskListSyncOutcome, DenError>;

    async fn request_task_list_handoff(
        &self,
        request: TaskListHandoffRequest,
    ) -> Result<TaskListHandoffOutcome, DenError>;
}

/// Postgres-backed `DocketService`. Holds the shared Den pool (cheap to clone —
/// it is an `Arc` internally).
#[derive(Debug, Clone)]
pub struct PgDocketService {
    pub(crate) pool: PgPool,
}

impl PgDocketService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Construct from a borrowed pool (clones the inner `Arc`).
    pub fn from_pool(pool: &PgPool) -> Self {
        Self { pool: pool.clone() }
    }
}

impl DocketService for PgDocketService {
    async fn create_job(&self, create: DocketJobCreate) -> Result<DocketJobProjection, DenError> {
        db::create_job(&self.pool, create).await
    }

    async fn list_jobs(
        &self,
        bear_id: Uuid,
        filter: DocketJobListFilter,
    ) -> Result<Vec<DocketJobRow>, DenError> {
        db::list_jobs(&self.pool, bear_id, filter).await
    }

    async fn get_job(
        &self,
        bear_id: Uuid,
        job_id: Uuid,
    ) -> Result<Option<DocketJobProjection>, DenError> {
        db::get_job(&self.pool, bear_id, job_id).await
    }

    async fn update_job(&self, update: DocketJobUpdate) -> Result<DocketJobProjection, DenError> {
        db::update_job(&self.pool, update).await
    }

    async fn evaluate_criterion(
        &self,
        update: DocketCriterionStateUpdate,
    ) -> Result<DocketJobProjection, DenError> {
        db::evaluate_criterion(&self.pool, update).await
    }

    async fn execute_job(
        &self,
        request: DocketJobExecuteRequest,
    ) -> Result<DocketJobExecuteOutcome, DenError> {
        db::execute_job(&self.pool, request).await
    }

    async fn get_active_execution_session(
        &self,
        bear_id: Uuid,
        owner_profile: BearProfile,
        lookup: DocketExecutionLookup,
    ) -> Result<Option<DocketExecutionSessionRow>, DenError> {
        db::get_active_execution_session(&self.pool, bear_id, owner_profile, lookup).await
    }

    async fn create_task(&self, create: DocketTaskCreate) -> Result<DocketTaskRow, DenError> {
        db::create_task(&self.pool, create).await
    }

    async fn list_tasks(
        &self,
        bear_id: Uuid,
        filter: DocketTaskListFilter,
    ) -> Result<Vec<DocketTaskProjection>, DenError> {
        db::list_tasks(&self.pool, bear_id, filter).await
    }

    async fn update_task(
        &self,
        update: DocketTaskUpdate,
    ) -> Result<DocketTaskProjection, DenError> {
        db::update_task(&self.pool, update).await
    }

    async fn checkout_task_list(
        &self,
        bear_id: Uuid,
        _viewer_role: BearProfile,
        _user_id: i32,
        request: TaskListCheckoutRequest,
    ) -> Result<Option<TaskListProjection>, DenError> {
        match request.source {
            TaskListCheckoutSource::DocketJob {
                job_id,
                parent_task_id,
            } => Ok(self
                .get_job(bear_id, job_id)
                .await?
                .map(|job| task_list_projection_from_docket_job(&job, parent_task_id))),
            TaskListCheckoutSource::LegacyWorkPlan(_) => Ok(None),
            TaskListCheckoutSource::LocalProjection(task_list) => Ok(Some(task_list)),
        }
    }

    async fn sync_task_list(
        &self,
        request: TaskListSyncRequest,
    ) -> Result<TaskListSyncOutcome, DenError> {
        db::sync_task_list(&self.pool, request).await
    }

    async fn request_task_list_handoff(
        &self,
        request: TaskListHandoffRequest,
    ) -> Result<TaskListHandoffOutcome, DenError> {
        Ok(TaskListHandoffOutcome::review_required(
            &request,
            "Task-list handoff seam is present; durable Docket promotion awaits relational Docket backing.",
        ))
    }
}
