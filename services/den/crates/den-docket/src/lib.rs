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
    BearWorkPlanRow, TaskListCheckoutRequest, TaskListCheckoutSource, TaskListHandoffOutcome,
    TaskListHandoffRequest, TaskListItem, TaskListProjection, TaskListSourceRef,
    TaskListSyncOutcome, TaskListSyncRequest, TaskListSyncState, WorkPlanItem, WorkPlanItemStatus,
    WorkPlanListFilter, WorkPlanLookup, WorkPlanProjection, WorkPlanStatus, WorkPlanUpdate,
    WorkPlanUpsert, WorkPlanValidationError, WorkPlanVisibility, normalize_work_plan_item_ids,
    render_workboard_prompt_context, role_can_read_work_plan, role_can_request_work_handoff,
    role_can_update_work_plan, task_list_item_from_work_plan_item,
    task_list_projection_from_work_plan, validate_work_plan_items, validate_work_plan_update,
};
pub use service::{DocketService, PgDocketService};
