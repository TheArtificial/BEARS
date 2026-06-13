//! `den`-side wiring for the memory review/curation tools and observations.
//!
//! Orchestration (gating, validation, projection-scope computation) lives in
//! `den-tools`; this module provides the concrete [`MemoryReviewStore`] —
//! composing proposal/observation persistence, the native/legacy core-update
//! paths, the memory-curate enqueue, and `conversation_events` projections —
//! wired into the dispatcher via `DenToolContext`.

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_tools::review::{
    ApplyCoreUpdateRequest, MemoryReviewStore, ObservationRecord, ObservationWriteRequest,
    ProposalProjection, RequestReviewRequest, ResolveProposalRequest,
};

use crate::{
    config::Config,
    core::{
        bear_observations::{self, BearObservationRow},
        bears::BearProfile,
        conversation_events::{
            memory_proposal_resolved_projection, memory_review_requested_projection,
            project_to_conversation, ProjectionProvenance, ProjectionSource,
        },
        memory::{
            create_observation, create_proposal, get_observation,
            get_proposal as db_get_proposal, list_proposals as db_list_proposals,
            mark_observation_review_queued_for_bear, promote_core_content,
            resolve_proposal as db_resolve_proposal, MemoryStoreManager,
        },
        memory_manager_head::{write_memfs_core_update, MemfsCoreUpdateRequest},
        memory_proposals::{CreateMemoryProposal, MemoryProposalRow, ProposalResolutionParams},
        reflection_conductor::{self, ProposalEnqueueParams},
        tools::memfs::memfs_http_client,
    },
    errors::{CustomError, DenError},
};

fn observation_record(row: &BearObservationRow) -> ObservationRecord {
    ObservationRecord {
        bear_id: row.bear_id,
        observation_id: row.observation_id.clone(),
        summary: row.summary.clone(),
        salience: row.salience.clone(),
        payload_ref: row.payload_ref.clone(),
        logical_path: row.logical_path.clone(),
        status: row.status.clone(),
        proposal_id: row.proposal_id,
    }
}

fn observation_requires_human(salience: &str) -> bool {
    matches!(salience, "high" | "critical")
}

/// Concrete [`MemoryReviewStore`] over the runtime pool/config/stores.
pub(crate) struct DenMemoryReviewStore<'a> {
    pool: &'a PgPool,
    config: &'a Config,
    stores: &'a MemoryStoreManager,
}

impl<'a> DenMemoryReviewStore<'a> {
    pub(crate) fn new(
        pool: &'a PgPool,
        config: &'a Config,
        stores: &'a MemoryStoreManager,
    ) -> Self {
        Self {
            pool,
            config,
            stores,
        }
    }

    fn provenance(&self, projection: &ProposalProjection) -> ProjectionProvenance {
        ProjectionProvenance {
            source: ProjectionSource::DenTools,
            scope_id: projection.scope_id.clone(),
        }
    }

    fn project_resolved(&self, projection: &ProposalProjection, resolved: &MemoryProposalRow) {
        project_to_conversation(
            self.pool,
            resolved.bear_id,
            Some(projection.user_id),
            projection.conversation_id.as_deref(),
            memory_proposal_resolved_projection(
                self.provenance(projection),
                resolved.id,
                resolved.source_profile.clone(),
                resolved.suggested_action.clone(),
                resolved.title.clone(),
                resolved.status.clone(),
                resolved.reviewer_profile.clone(),
                resolved.result_path.clone(),
                resolved.result_commit.clone(),
            ),
        );
    }

    async fn enqueue_observation_review(
        &self,
        request: &ObservationWriteRequest,
        observation: &BearObservationRow,
        salience: &str,
    ) -> Result<MemoryProposalRow, DenError> {
        let requires_human = observation_requires_human(salience);
        let conversation_id = request.conversation_id.clone();
        let proposal = create_proposal(
            self.pool,
            self.config,
            self.stores,
            CreateMemoryProposal {
                bear_id: request.bear_id,
                source_profile: BearProfile::Watch,
                source_agent_id: Some(request.binding_id.clone()),
                source_paths: vec![observation.logical_path.clone()],
                source_refs: json!({
                    "observation_id": observation.observation_id,
                    "observation_row_id": observation.id,
                    "conversation_id": conversation_id,
                    "session_id": request.session_id,
                    "request_id": request.request_id,
                }),
                suggested_action: if requires_human {
                    "human_review"
                } else {
                    "unspecified"
                },
                target_ref: None,
                title: &format!("Review watch observation: {}", observation.observation_id),
                summary: observation.summary.as_str(),
                rationale: "Watch recorded an inbound observation that may warrant curate review.",
                proposed_content: None,
                proposed_patch: None,
                refs: json!({
                    "observation_id": observation.observation_id,
                    "salience": salience,
                    "payload_ref": observation.payload_ref,
                    "logical_path": observation.logical_path,
                }),
                sensitivity: "normal",
                requires_human,
                project_to_conversation: conversation_id.is_some(),
            },
        )
        .await
        .map_err(CustomError::into_den)?;

        let reflection_date = OffsetDateTime::now_utc().date();
        let conversation_key = format!("memory_curate:{reflection_date}");
        reflection_conductor::enqueue_memory_curate_for_proposals(
            self.pool,
            ProposalEnqueueParams {
                bear_id: request.bear_id,
                binding_id: Some(request.binding_id.as_str()),
                conversation_id: conversation_id.as_deref(),
                conversation_key: Some(&conversation_key),
                conversation_date: Some(reflection_date),
                trigger: "watch_observation",
                proposal_ids: vec![proposal.id],
            },
        )
        .await
        .map_err(CustomError::into_den)?;

        Ok(proposal)
    }
}

#[async_trait]
impl MemoryReviewStore for DenMemoryReviewStore<'_> {
    async fn find_observation(
        &self,
        bear_id: Uuid,
        observation_id: &str,
    ) -> Result<Option<ObservationRecord>, DenError> {
        let existing = get_observation(self.pool, self.config, self.stores, bear_id, observation_id)
            .await
            .map_err(CustomError::into_den)?;
        Ok(existing.as_ref().map(observation_record))
    }

    async fn record_observation(
        &self,
        request: ObservationWriteRequest,
    ) -> Result<ObservationRecord, DenError> {
        let salience = request.salience.clone();
        let observation = create_observation(
            self.pool,
            self.config,
            self.stores,
            bear_observations::CreateBearObservation {
                bear_id: request.bear_id,
                observation_id: &request.observation_id,
                summary: &request.summary,
                salience: &salience,
                payload_ref: request.payload_ref.as_deref(),
                source: request.source.clone(),
            },
        )
        .await
        .map_err(CustomError::into_den)?;

        let proposal = self
            .enqueue_observation_review(&request, &observation, &salience)
            .await?;

        if self.config.uses_native_agent_runtime() {
            mark_observation_review_queued_for_bear(
                self.config,
                self.stores,
                request.bear_id,
                &observation.observation_id,
                proposal.id,
            )
            .await
            .map_err(CustomError::into_den)?;
            let mut observation = observation;
            observation.status = "review_queued".to_string();
            observation.proposal_id = Some(proposal.id);
            return Ok(observation_record(&observation));
        }

        let observation = bear_observations::mark_review_queued(
            self.pool,
            request.bear_id,
            observation.id,
            proposal.id,
        )
        .await
        .map_err(CustomError::into_den)?;
        Ok(observation_record(&observation))
    }

    async fn list_proposals(
        &self,
        bear_id: Uuid,
        status: Option<String>,
        limit: i64,
    ) -> Result<Value, DenError> {
        let proposals =
            db_list_proposals(self.pool, self.config, self.stores, bear_id, status.as_deref(), limit)
                .await
                .map_err(CustomError::into_den)?;
        Ok(json!(proposals))
    }

    async fn get_proposal(
        &self,
        bear_id: Uuid,
        proposal_id: Uuid,
    ) -> Result<Option<Value>, DenError> {
        let proposal = db_get_proposal(self.pool, self.config, self.stores, bear_id, proposal_id)
            .await
            .map_err(CustomError::into_den)?;
        Ok(proposal.map(|proposal| json!(proposal)))
    }

    async fn resolve_proposal(&self, request: ResolveProposalRequest) -> Result<Value, DenError> {
        let resolved = db_resolve_proposal(
            self.pool,
            self.config,
            self.stores,
            ProposalResolutionParams {
                bear_id: request.bear_id,
                proposal_id: request.proposal_id,
                reviewer_profile: request.reviewer_profile,
                reviewer_agent_id: Some(request.binding_id.as_str()),
                status: &request.status,
                review_notes: request.review_notes.as_deref(),
                decision_summary: request.decision_summary.as_deref(),
                result_path: None,
                result_commit: None,
                project_to_conversation: false,
            },
        )
        .await
        .map_err(CustomError::into_den)?;
        self.project_resolved(&request.projection, &resolved);
        Ok(json!(resolved))
    }

    async fn request_review(&self, request: RequestReviewRequest) -> Result<Value, DenError> {
        let proposal = create_proposal(
            self.pool,
            self.config,
            self.stores,
            CreateMemoryProposal {
                bear_id: request.bear_id,
                source_profile: request.source_profile,
                source_agent_id: request.binding_id.clone(),
                source_paths: request.source_paths.clone(),
                source_refs: request.source_refs.clone(),
                suggested_action: &request.suggested_action,
                target_ref: request.target_ref.as_deref(),
                title: &request.title,
                summary: &request.summary,
                rationale: &request.rationale,
                proposed_content: request.proposed_content.as_deref(),
                proposed_patch: request.proposed_patch.as_deref(),
                refs: request.refs.clone(),
                sensitivity: &request.sensitivity,
                requires_human: request.requires_human,
                project_to_conversation: false,
            },
        )
        .await
        .map_err(CustomError::into_den)?;
        project_to_conversation(
            self.pool,
            proposal.bear_id,
            Some(request.projection.user_id),
            request.projection.conversation_id.as_deref(),
            memory_review_requested_projection(
                self.provenance(&request.projection),
                proposal.id,
                proposal.source_profile.clone(),
                proposal.suggested_action.clone(),
                proposal.title.clone(),
                proposal.status.clone(),
                proposal.source_paths.clone(),
            ),
        );
        Ok(json!(proposal))
    }

    async fn apply_core_update(&self, request: ApplyCoreUpdateRequest) -> Result<Value, DenError> {
        let proposal = db_get_proposal(
            self.pool,
            self.config,
            self.stores,
            request.bear_id,
            request.proposal_id,
        )
        .await
        .map_err(CustomError::into_den)?
        .ok_or_else(|| DenError::NotFound("memory proposal not found".to_string()))?;

        if self.config.uses_native_agent_runtime() {
            let content = request.body.clone().unwrap_or_else(|| {
                format!(
                    "Applied from proposal `{}` via native SQLite promotion.",
                    proposal.id
                )
            });
            let kind = request
                .target_path
                .split('/')
                .next_back()
                .unwrap_or("note")
                .trim_end_matches(".md");
            let (memory_id, promotion_id) = promote_core_content(
                self.stores,
                request.bear_id,
                &proposal.id.to_string(),
                kind,
                &content,
                request.reviewer_profile.as_str(),
            )
            .await
            .map_err(CustomError::into_den)?;
            let resolved = db_resolve_proposal(
                self.pool,
                self.config,
                self.stores,
                ProposalResolutionParams {
                    bear_id: request.bear_id,
                    proposal_id: proposal.id,
                    reviewer_profile: request.reviewer_profile,
                    reviewer_agent_id: Some(request.binding_id.as_str()),
                    status: "approved",
                    review_notes: request.review_notes.as_deref(),
                    decision_summary: Some("Applied reviewed memory proposal to core (SQLite)."),
                    result_path: Some(request.target_path.as_str()),
                    result_commit: None,
                    project_to_conversation: false,
                },
            )
            .await
            .map_err(CustomError::into_den)?;
            self.project_resolved(&request.projection, &resolved);
            return Ok(json!({
                "bear_id": request.bear_id,
                "proposal": resolved,
                "core_update": {
                    "path": request.target_path,
                    "memory_id": memory_id,
                    "promotion_id": promotion_id,
                },
            }));
        }

        let http = memfs_http_client("MemFS core update client build failed")
            .map_err(CustomError::into_den)?;
        let body = request.body.clone().map(|body| {
            format!(
                "{}\n\n---\nSource proposal: `{}`\nSource role: `{}`\nSource paths: {}\n",
                body.trim(),
                proposal.id,
                proposal.source_profile,
                proposal.source_paths.join(", ")
            )
        });
        let core_request = MemfsCoreUpdateRequest {
            target_path: request.target_path.clone(),
            mode: request.mode.clone(),
            title: request.title.clone().or_else(|| Some(proposal.title.clone())),
            body,
            old_text: request.old_text.clone(),
            new_text: request.new_text.clone(),
            proposal_id: Some(proposal.id),
            source_paths: proposal.source_paths.clone(),
        };
        let response = write_memfs_core_update(
            &http,
            &self.config.letta_memfs_service_url,
            request.bear_id,
            &core_request,
        )
        .await
        .map_err(CustomError::into_den)?;
        let Some(response) = response else {
            return Err(DenError::System(
                "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)".to_string(),
            ));
        };
        let resolved = db_resolve_proposal(
            self.pool,
            self.config,
            self.stores,
            ProposalResolutionParams {
                bear_id: request.bear_id,
                proposal_id: proposal.id,
                reviewer_profile: request.reviewer_profile,
                reviewer_agent_id: Some(request.binding_id.as_str()),
                status: "approved",
                review_notes: request.review_notes.as_deref(),
                decision_summary: Some("Applied reviewed memory proposal to core."),
                result_path: Some(response.path.as_str()),
                result_commit: response.canonical_tip.as_deref(),
                project_to_conversation: false,
            },
        )
        .await
        .map_err(CustomError::into_den)?;
        self.project_resolved(&request.projection, &resolved);
        Ok(json!({
            "bear_id": request.bear_id,
            "proposal": resolved,
            "core_update": response,
        }))
    }
}

