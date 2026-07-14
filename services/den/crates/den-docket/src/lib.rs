//! `den-docket` — Den's control-plane subsystem for work management (ADR-0034).
//!
//! Public face is [`DocketService`] / [`PgDocketService`]; `db` is crate-internal.
//! Docket jobs/tasks use the ADR-0034 relational Postgres tables. Historical
//! task-list tables may exist in old migrations/data, but this crate should not
//! keep active read/write shims for them.
//! This crate is a service-layer leaf: it depends only on `den-core`. The
//! relational jobs/tasks realization is tracked in
//! `docs/roadmap/DOCKET_IMPLEMENTATION_PLAN.md`; the crate split itself in
//! `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

mod db;
mod dispatcher;
#[cfg(test)]
mod integration_tests;
pub mod model;
pub mod service;
pub mod work_runs;
#[cfg(test)]
mod work_runs_tests;

pub use dispatcher::TaskDispatcher;
pub use model::{
    docket_job_status_report, docket_task_status_from_task_list_item_status,
    normalize_task_list_item_ids, render_task_list_prompt_context, role_can_read_task_list,
    role_can_request_task_list_handoff, role_can_update_task_list, task_list_item_from_update_item,
    task_list_projection_from_docket_job, task_list_projection_from_local,
    task_list_projection_from_session_tasks, validate_docket_job_create, validate_docket_task_create,
    validate_task_list_items, validate_task_list_update, DocketCommitPolicy, DocketCountByStatus,
    DocketCriteriaCountByStatus, DocketCriterionKind, DocketCriterionStateRow,
    DocketCriterionStateUpdate, DocketCriterionStatus, DocketEffortHint, DocketExecutionLookup,
    DocketExecutionSessionRow, DocketExecutionSessionUpsert, DocketJobCreate,
    DocketJobCriterionInput, DocketJobCriterionRow, DocketJobExecuteOutcome,
    DocketJobExecuteRequest, DocketJobListFilter, DocketJobProjection, DocketJobRow,
    DocketJobRunRow, DocketJobStatus, DocketJobStatusReport, DocketJobUpdate, DocketRunState,
    DocketRunTrigger, DocketTaskCreate, DocketTaskDefinitionPatch, DocketTaskDifficulty,
    DocketTaskInput, DocketTaskKind, DocketTaskListFilter, DocketTaskProjection, DocketTaskRow,
    DocketTaskRunStateRow, DocketTaskRunStateUpdate, DocketTaskScope, DocketTaskStatus,
    DocketTaskUpdate, DocketValidationError, TaskListCheckoutRequest, TaskListCheckoutSource,
    TaskListHandoffOutcome, TaskListHandoffRequest, TaskListItem, TaskListItemStatus,
    TaskListLocalProjection, TaskListProjection, TaskListSourceRef, TaskListStatus,
    TaskListSyncOutcome, TaskListSyncRequest, TaskListSyncState, TaskListUpdate,
    TaskListUpdateItem, TaskListValidationError, TaskListVisibility,
};
pub use service::{DocketService, PgDocketService};
