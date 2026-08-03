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
        persist_recognized_session_surface(self.pool, self.stores, context, role).await?;
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

fn normalized_git_remote(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/').trim_end_matches(".git");
    let value = value
        .strip_prefix("git@")
        .and_then(|value| {
            value
                .split_once(':')
                .map(|(host, path)| format!("{host}/{path}"))
        })
        .or_else(|| {
            value.split_once("://").map(|(_, authority_and_path)| {
                authority_and_path
                    .strip_prefix("git@")
                    .unwrap_or(authority_and_path)
                    .to_string()
            })
        })
        .unwrap_or_else(|| value.to_string());
    (!value.is_empty()).then(|| value.to_ascii_lowercase())
}

fn unique_git_remote_match_index<'a>(
    origins: &[String],
    upstreams: impl Iterator<Item = &'a str>,
) -> Option<usize> {
    let mut matches = upstreams.enumerate().filter_map(|(index, upstream)| {
        normalized_git_remote(upstream)
            .is_some_and(|upstream| origins.iter().any(|origin| origin == &upstream))
            .then_some(index)
    });
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn session_git_remote_origins(
    session: &den_service::client_sessions::ClientSessionRow,
) -> Vec<String> {
    session
        .adapter_environment
        .as_ref()
        .and_then(|environment| environment.get("git_remote_origins"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(normalized_git_remote)
        .collect()
}

pub(crate) async fn persist_recognized_session_surface(
    pool: &PgPool,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearProfile,
) -> Result<(), DenError> {
    let Some(client_session_id) = context.client_session_id.as_deref() else {
        return Ok(());
    };
    let Some(session) = den_service::client_sessions::find_for_user_bear_session_id(
        pool,
        context.user_id,
        context.bear_id,
        client_session_id,
    )
    .await?
    else {
        return Ok(());
    };
    if den_service::client_session_work_surface_resolutions::find(pool, session.id)
        .await?
        .is_some_and(|resolution| resolution.status == "confirmed")
    {
        return Ok(());
    }
    let assigned_surfaces =
        den_service::work_surfaces::list_surfaces_for_bears(pool, &[context.bear_id]).await?;
    let session_origins = session_git_remote_origins(&session);
    if let Some(match_index) = unique_git_remote_match_index(
        &session_origins,
        assigned_surfaces
            .iter()
            .map(|surface| surface.upstream_url.as_str()),
    ) {
        den_service::client_session_work_surface_resolutions::upsert(
            pool,
            session.id,
            assigned_surfaces[match_index].id,
            den_service::client_session_work_surface_resolutions::ClientSessionWorkSurfaceResolutionStatus::Resolved,
            json!({"kind": "git_remote_origin", "origins": session_origins}),
        )
        .await?;
        return Ok(());
    }
    let hint_payload = infer_work_surface_hint(context, role);
    let candidate_slug = work_surface_candidate_slug(context);
    let store = stores.store_for_bear(context.bear_id).await?;
    let files = sqlite_memory::sqlite_collect_role_logical_paths(&store, role.as_str()).await?;
    let orientation =
        build_work_surface_orientation_payload(role, &hint_payload, &files, candidate_slug);
    let candidate_slug = orientation
        .pointer("/work_surface/slug")
        .and_then(Value::as_str);
    let has_canonical_anchor = orientation
        .get("canonical_paths")
        .and_then(Value::as_array)
        .is_some_and(|paths| !paths.is_empty());
    if let Some(candidate_slug) = candidate_slug.filter(|_| has_canonical_anchor) {
        let matches = den_service::work_surfaces::list_surfaces_for_bears(pool, &[context.bear_id])
            .await?
            .into_iter()
            .filter(|surface| {
                normalize_work_surface_slug(&surface.name).ok().as_deref() == Some(candidate_slug)
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            den_service::client_session_work_surface_resolutions::upsert(
                pool,
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
            return Ok(());
        }
    }
    den_service::client_session_work_surface_resolutions::clear_resolved(pool, session.id).await?;
    Ok(())
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod git_remote_tests {
    use super::{normalized_git_remote, unique_git_remote_match_index};

    #[test]
    fn normalizes_equivalent_git_remote_forms() {
        let expected = Some("github.com/bears-ai/bear-den".to_string());
        assert_eq!(
            normalized_git_remote("git@github.com:bears-ai/bear-den.git"),
            expected
        );
        assert_eq!(
            normalized_git_remote("https://github.com/bears-ai/bear-den.git/"),
            Some("github.com/bears-ai/bear-den".to_string())
        );
        assert_eq!(
            normalized_git_remote("ssh://git@github.com/bears-ai/bear-den.git"),
            Some("github.com/bears-ai/bear-den".to_string())
        );
    }

    #[test]
    fn matches_only_one_assigned_surface_for_session_origins() {
        let origins = vec!["github.com/bears-ai/bear-den".to_string()];
        assert_eq!(
            unique_git_remote_match_index(
                &origins,
                [
                    "ssh://git@github.com/bears-ai/bear-den.git",
                    "https://github.com/bears-ai/other.git",
                ]
                .into_iter(),
            ),
            Some(0)
        );
        assert_eq!(
            unique_git_remote_match_index(
                &origins,
                [
                    "https://github.com/bears-ai/bear-den.git",
                    "git@github.com:bears-ai/bear-den.git",
                ]
                .into_iter(),
            ),
            None
        );
        assert_eq!(
            unique_git_remote_match_index(
                &origins,
                ["https://github.com/bears-ai/other.git"].into_iter(),
            ),
            None
        );
    }
}
