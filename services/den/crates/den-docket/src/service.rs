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
    BearWorkPlanRow, WorkPlanListFilter, WorkPlanLookup, WorkPlanProjection, WorkPlanUpsert,
};

/// Orchestration API for Docket work plans. The only public entry point to the
/// subsystem's persistence; never execute task bodies here (ADR-0034 execution
/// invariant) — Docket schedules, gates, and records.
// Native async fn in trait: workspace-internal, only ever consumed via generic
// bounds / the concrete `PgDocketService` (never `dyn`), so auto-trait (Send)
// bounds flow through monomorphization and async-trait boxing is unnecessary.
#[allow(async_fn_in_trait)]
pub trait DocketService: Send + Sync {
    async fn upsert_work_plan(&self, params: WorkPlanUpsert) -> Result<BearWorkPlanRow, DenError>;

    async fn list_visible_work_plans(
        &self,
        bear_id: Uuid,
        viewer_role: BearProfile,
        user_id: i32,
        filter: WorkPlanListFilter,
    ) -> Result<Vec<WorkPlanProjection>, DenError>;

    async fn get_visible_work_plan(
        &self,
        bear_id: Uuid,
        viewer_role: BearProfile,
        user_id: i32,
        lookup: WorkPlanLookup,
    ) -> Result<Option<WorkPlanProjection>, DenError>;
}

/// Postgres-backed `DocketService`. Holds the shared Den pool (cheap to clone —
/// it is an `Arc` internally).
#[derive(Debug, Clone)]
pub struct PgDocketService {
    pool: PgPool,
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
    async fn upsert_work_plan(&self, params: WorkPlanUpsert) -> Result<BearWorkPlanRow, DenError> {
        db::create_or_update_work_plan(&self.pool, params).await
    }

    async fn list_visible_work_plans(
        &self,
        bear_id: Uuid,
        viewer_role: BearProfile,
        user_id: i32,
        filter: WorkPlanListFilter,
    ) -> Result<Vec<WorkPlanProjection>, DenError> {
        db::list_visible_work_plans(&self.pool, bear_id, viewer_role, user_id, filter).await
    }

    async fn get_visible_work_plan(
        &self,
        bear_id: Uuid,
        viewer_role: BearProfile,
        user_id: i32,
        lookup: WorkPlanLookup,
    ) -> Result<Option<WorkPlanProjection>, DenError> {
        db::get_visible_work_plan(&self.pool, bear_id, viewer_role, user_id, lookup).await
    }
}
