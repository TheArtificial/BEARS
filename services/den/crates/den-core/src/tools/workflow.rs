//! Capability seam for the work-plan (activity) tools.
//!
//! Unlike the other Phase B groups, the work-plan executors are saturated with
//! `den`-only domain types (`work_plans::*`, `den_docket::DocketService`,
//! `acp_plan_mode`) plus the activity-payload `fn` builders, none of which live
//! in a shared crate yet (that is v0 de-stringify / docket-split work). Until
//! those types migrate, [`WorkPlanOps`] is a coarse seam: argument parsing and
//! response shaping stay in the `den` implementation, and the trait simply lets
//! the relocated dispatcher route the three work-plan tools through a capability
//! boundary. See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B — dispatcher).

use async_trait::async_trait;
use serde_json::Value;

use den_core::{BearProfile, DenError};

use crate::context::DenToolInvocationContext;

#[async_trait]
pub trait WorkPlanOps: Send + Sync {
    async fn list_work_plans(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;

    async fn get_work_plan_status(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;

    async fn update_work_plan(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;
}
