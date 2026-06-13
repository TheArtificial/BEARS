//! `den`-side wiring for `observation_write`.
//!
//! Orchestration (watch gating, validation, idempotency, payload shaping) lives
//! in `den-tools`; here we provide the concrete [`MemoryReviewStore`] that
//! composes observation persistence, proposal creation, and the memory-curate
//! enqueue, and a thin wrapper that maps `DenError` back to `CustomError`.

use async_trait::async_trait;
use time::OffsetDateTime;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use den_tools::review::{
    MemoryReviewStore, ObservationRecord, ObservationWriteRequest,
};

use crate::{
    config::Config,
    core::{
        bear_observations::{self, BearObservationRow},
        bears::BearProfile,
        memory::{
            create_observation, create_proposal, get_observation,
            mark_observation_review_queued_for_bear, MemoryStoreManager,
        },
        memory_proposals::{self, CreateMemoryProposal},
        reflection_conductor::{self, ProposalEnqueueParams},
        tools::session::DenToolInvocationContext,
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
struct DenMemoryReviewStore<'a> {
    pool: &'a PgPool,
    config: &'a Config,
    stores: &'a MemoryStoreManager,
}

impl DenMemoryReviewStore<'_> {
    async fn enqueue_observation_review(
        &self,
        request: &ObservationWriteRequest,
        observation: &BearObservationRow,
        salience: &str,
    ) -> Result<memory_proposals::MemoryProposalRow, DenError> {
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
                source_refs: serde_json::json!({
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
                refs: serde_json::json!({
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

        let observation =
            bear_observations::mark_review_queued(self.pool, request.bear_id, observation.id, proposal.id)
                .await
                .map_err(CustomError::into_den)?;
        Ok(observation_record(&observation))
    }
}

pub(crate) async fn write_observation(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let store = DenMemoryReviewStore {
        pool,
        config,
        stores,
    };
    den_tools::review::write_observation(&store, context, role, arguments)
        .await
        .map_err(CustomError::from)
}
