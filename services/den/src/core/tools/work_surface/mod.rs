//! `den`-side wiring for the work-surface tools.
//!
//! All builders/orientation helpers now live in `den-tools::work_surface` and are
//! re-exported here so existing `core::tools::work_surface::*` paths keep
//! resolving. This module provides the concrete [`WorkSurfaceOps`] (native SQLite
//! scaffold writes) and the thin `create_work_surface_scaffold` wrapper. See
//! `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B, ADR-0040).

pub(crate) use den_core::tools::work_surface::*;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::tools::work_surface::{ScaffoldRequest, WorkSurfaceOps, WorkSurfaceScaffoldOutcome};

use crate::{config::Config, core::tools::session::DenToolInvocationContext, errors::DenError};
use den_memory::{tools as sqlite_memory, MemoryStoreManager};
use den_service::bears::BearProfile;

/// Concrete [`WorkSurfaceOps`] over the runtime memory stores.
pub(crate) struct DenWorkSurfaceOps<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) config: &'a Config,
    pub(crate) stores: &'a MemoryStoreManager,
}

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
            .await?;
            responses.push(written);
        }
        // Async-index these scaffold writes into derived recall (ADR-0038 Phase 1b);
        // best-effort, coalesced per Bear.
        den_runtime::reflection_conductor::enqueue_recall_index_if_enabled(
            self.pool,
            self.config,
            bear_id,
            "work_surface_scaffold",
        )
        .await;
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
        let files = sqlite_memory::sqlite_collect_role_logical_paths(&store, role.as_str()).await?;
        let orientation =
            build_work_surface_orientation_payload(role, &hint_payload, &files, candidate_slug);
        if let Some(client_session_id) = context.client_session_id.as_deref() {
            let session = den_service::client_sessions::find_for_user_bear_session_id(
                self.pool,
                context.user_id,
                context.bear_id,
                client_session_id,
            )
            .await?;
            if let Some(session) = session {
                let candidate_slug = orientation
                    .pointer("/work_surface/slug")
                    .and_then(Value::as_str);
                let has_canonical_anchor = orientation
                    .get("canonical_paths")
                    .and_then(Value::as_array)
                    .is_some_and(|paths| !paths.is_empty());
                if let Some(candidate_slug) = candidate_slug.filter(|_| has_canonical_anchor) {
                    let matches = den_service::work_surfaces::list_surfaces_for_bears(
                        self.pool,
                        &[context.bear_id],
                    )
                    .await?
                    .into_iter()
                    .filter(|surface| {
                        normalize_work_surface_slug(&surface.name).ok().as_deref()
                            == Some(candidate_slug)
                    })
                    .collect::<Vec<_>>();
                    if matches.len() == 1 {
                        den_service::client_session_work_surface_resolutions::upsert(
                            self.pool,
                            session.id,
                            matches[0].id,
                            den_service::client_session_work_surface_resolutions::ClientSessionWorkSurfaceResolutionStatus::Resolved,
                            json!({
                                "kind": "memory_anchor",
                                "candidate_slug": candidate_slug,
                                "canonical_anchor_paths": orientation.get("canonical_paths"),
                            }),
                        )
                        .await?;
                    } else {
                        den_service::client_session_work_surface_resolutions::clear_resolved(
                            self.pool, session.id,
                        )
                        .await?;
                    }
                } else {
                    den_service::client_session_work_surface_resolutions::clear_resolved(
                        self.pool, session.id,
                    )
                    .await?;
                }
            }
        }
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
