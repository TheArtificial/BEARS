//! `den`-side wiring for the work-surface tools.
//!
//! All builders/orientation helpers now live in `den-tools::work_surface` and are
//! re-exported here so existing `core::tools::work_surface::*` paths keep
//! resolving. This module provides the concrete [`WorkSurfaceOps`] (native SQLite
//! scaffold writes) and the thin `create_work_surface_scaffold` wrapper. See
//! `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B, ADR-0040).

pub(crate) use den_tools::work_surface::*;

use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use den_tools::work_surface::{ScaffoldRequest, WorkSurfaceOps, WorkSurfaceScaffoldOutcome};

use crate::{
    core::{
        bears::BearProfile,
        memory::{tools as sqlite_memory, MemoryStoreManager},
        tools::session::DenToolInvocationContext,
    },
    errors::DenError,
};

/// Concrete [`WorkSurfaceOps`] over the runtime memory stores.
pub(crate) struct DenWorkSurfaceOps<'a> {
    pub(crate) stores: &'a MemoryStoreManager,
}

#[async_trait]
impl WorkSurfaceOps for DenWorkSurfaceOps<'_> {
    async fn write_scaffold(
        &self,
        bear_id: Uuid,
        role: BearProfile,
        slug: &str,
        name: &str,
        requests: Vec<ScaffoldRequest>,
    ) -> Result<WorkSurfaceScaffoldOutcome, DenError> {
        let mut responses = Vec::new();
        for request in requests {
            let body = request.body.as_deref().unwrap_or_default().to_string();
            let title = request.title.as_deref().unwrap_or(name);
            let written = sqlite_memory::sqlite_write_at_path(
                self.stores,
                bear_id,
                &request.target_path,
                role.as_str(),
                title,
                &body,
                json!({
                    "mode": request.mode,
                    "work_surface_slug": slug,
                }),
            )
            .await
            ?;
            responses.push(written);
        }
        Ok(WorkSurfaceScaffoldOutcome {
            storage: Some("sqlite".to_string()),
            updates: responses,
        })
    }

    async fn orient(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
    ) -> Result<Value, DenError> {
        let hint_payload = infer_work_surface_hint(context, role);
        let candidate_slug = work_surface_candidate_slug(context);
        let store = self.stores.store_for_bear(context.bear_id).await?;
        let files = sqlite_memory::sqlite_collect_role_logical_paths(&store, role.as_str())
            .await
            ?;
        let orientation =
            build_work_surface_orientation_payload(role, &hint_payload, &files, candidate_slug);
        Ok(json!({
            "ok": true,
            "configured": true,
            "storage": "sqlite",
            "bear_id": context.bear_id,
            "profile": role.as_str(),
            "orientation": orientation,
        }))
    }
}

#[cfg(test)]
mod test;
