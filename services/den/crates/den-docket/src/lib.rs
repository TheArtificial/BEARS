//! `den-docket` — Den's control-plane subsystem for work management (ADR-0034).
//!
//! Public face is [`DocketService`] / [`PgDocketService`]; `db` is crate-internal.
//! Today this wraps the **legacy `bear_work_plans` activity board** (JSONB
//! items); see [`model`] for why the types keep their honest pre-ADR-0034 names.
//! This crate is a service-layer leaf: it depends only on `den-core`. The
//! relational jobs/tasks realization is tracked in
//! `docs/roadmap/DOCKET_IMPLEMENTATION_PLAN.md`; the crate split itself in
//! `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

mod db;
pub mod model;
pub mod service;

pub use model::{
    docket_job_status_report, docket_task_status_from_work_plan_item_status,
    normalize_work_plan_item_ids, render_workboard_prompt_context, role_can_read_work_plan,
    role_can_request_work_handoff, role_can_update_work_plan, task_list_item_from_work_plan_item,
    task_list_projection_from_docket_job, task_list_projection_from_work_plan,
    validate_docket_job_create, validate_docket_task_create, validate_work_plan_items,
    validate_work_plan_update, BearWorkPlanRow, DocketCommitPolicy, DocketCountByStatus,
    DocketCriteriaCountByStatus, DocketCriterionKind, DocketCriterionStateRow,
    DocketCriterionStateUpdate, DocketCriterionStatus, DocketEffortHint, DocketJobCreate,
    DocketJobCriterionInput, DocketJobCriterionRow, DocketJobExecuteOutcome,
    DocketJobExecuteRequest, DocketJobListFilter, DocketJobProjection, DocketJobRow,
    DocketJobRunRow, DocketJobStatus, DocketJobStatusReport, DocketJobUpdate, DocketRunState,
    DocketRunTrigger, DocketTaskCreate, DocketTaskDefinitionPatch, DocketTaskDifficulty,
    DocketTaskInput, DocketTaskKind, DocketTaskListFilter, DocketTaskProjection, DocketTaskRow,
    DocketTaskRunStateRow, DocketTaskRunStateUpdate, DocketTaskScope, DocketTaskStatus,
    DocketTaskUpdate, DocketValidationError, TaskListCheckoutRequest, TaskListCheckoutSource,
    TaskListHandoffOutcome, TaskListHandoffRequest, TaskListItem, TaskListProjection,
    TaskListSourceRef, TaskListSyncOutcome, TaskListSyncRequest, TaskListSyncState, WorkPlanItem,
    WorkPlanItemStatus, WorkPlanListFilter, WorkPlanLookup, WorkPlanProjection, WorkPlanStatus,
    WorkPlanUpdate, WorkPlanUpsert, WorkPlanValidationError, WorkPlanVisibility,
};
pub use service::{DocketService, PgDocketService};
