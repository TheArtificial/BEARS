//! `den`-side wiring for the work-surface tools.
//!
//! All builders/orientation helpers now live in `den-tools::work_surface` and are
//! re-exported here so existing `core::tools::work_surface::*` paths keep
//! resolving. This module provides the concrete [`WorkSurfaceOps`] (native SQLite
//! + legacy MemFS scaffold writes) and the thin `create_work_surface_scaffold`
//! wrapper. See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B, ADR-0040).

pub(crate) use den_tools::work_surface::*;

use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use den_tools::work_surface::{
    work_surface_entry_body, ScaffoldRequest, WorkSurfaceOps, WorkSurfaceScaffoldOutcome,
};

use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        memory::{tools as sqlite_memory, MemoryStoreManager},
        memory_manager_head::{
            append_markdown_section, fetch_memfs_role_memory_file, write_memfs_core_update,
            MemfsCoreUpdateRequest,
        },
        tools::{memfs::memfs_http_client, session::DenToolInvocationContext},
    },
    errors::{CustomError, DenError},
};

fn scaffold_to_core_update(request: ScaffoldRequest) -> MemfsCoreUpdateRequest {
    MemfsCoreUpdateRequest {
        target_path: request.target_path,
        mode: request.mode,
        title: request.title,
        body: request.body,
        old_text: request.old_text,
        new_text: request.new_text,
        proposal_id: None,
        source_paths: vec![],
    }
}

/// Concrete [`WorkSurfaceOps`] over the runtime config/stores.
struct DenWorkSurfaceOps<'a> {
    config: &'a Config,
    stores: &'a MemoryStoreManager,
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
        if self.config.uses_native_agent_runtime() {
            let mut responses = Vec::new();
            for request in requests {
                let body = request.body.as_deref().unwrap_or_default().to_string();
                let title = request.title.as_deref().unwrap_or(name);
                let written = sqlite_memory::sqlite_write_at_path(
                    self.stores,
                    self.config,
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
                .map_err(CustomError::into_den)?;
                responses.push(written);
            }
            return Ok(WorkSurfaceScaffoldOutcome {
                storage: Some("sqlite".to_string()),
                updates: responses,
            });
        }

        let http = memfs_http_client("MemFS work-surface scaffold client build failed")
            .map_err(CustomError::into_den)?;
        let mut responses = Vec::new();
        for request in requests {
            if request.target_path == "core/work_surfaces/index.md"
                && request.mode == "append_section"
            {
                let registry = fetch_memfs_role_memory_file(
                    &http,
                    &self.config.letta_memfs_service_url,
                    bear_id,
                    role.as_str(),
                    "core/work_surfaces/index.md",
                )
                .await
                .map_err(CustomError::into_den)?;
                let existing = registry.map(|file| file.content).unwrap_or_default();
                let updated = append_markdown_section(
                    &existing,
                    &format!("## {name}"),
                    &work_surface_entry_body(slug, name),
                );
                let existing_is_empty = existing.trim().is_empty();
                let replace_request = MemfsCoreUpdateRequest {
                    target_path: "core/work_surfaces/index.md".to_string(),
                    mode: if existing_is_empty {
                        "create_file".to_string()
                    } else {
                        "replace_text".to_string()
                    },
                    title: Some("Work Surfaces".to_string()),
                    body: if existing_is_empty {
                        Some(updated.clone())
                    } else {
                        None
                    },
                    old_text: if existing_is_empty { None } else { Some(existing) },
                    new_text: if existing_is_empty { None } else { Some(updated) },
                    proposal_id: None,
                    source_paths: vec![],
                };
                let response = write_memfs_core_update(
                    &http,
                    &self.config.letta_memfs_service_url,
                    bear_id,
                    &replace_request,
                )
                .await
                .map_err(CustomError::into_den)?;
                if let Some(response) = response {
                    responses.push(json!(response));
                }
                continue;
            }
            let response = write_memfs_core_update(
                &http,
                &self.config.letta_memfs_service_url,
                bear_id,
                &scaffold_to_core_update(request),
            )
            .await
            .map_err(CustomError::into_den)?;
            if let Some(response) = response {
                responses.push(json!(response));
            }
        }
        Ok(WorkSurfaceScaffoldOutcome {
            storage: None,
            updates: responses,
        })
    }
}

pub(crate) async fn create_work_surface_scaffold(
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let ops = DenWorkSurfaceOps { config, stores };
    den_tools::work_surface::create_work_surface_scaffold(&ops, context, role, arguments)
        .await
        .map_err(CustomError::from)
}

#[cfg(test)]
mod test;
