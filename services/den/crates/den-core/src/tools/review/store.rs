//! The `MemoryReviewStore` capability seam: reflection/curation review surface
//! (observations + proposals + enqueue + projections).
//!
//! The granular den proposal/observation row + param types carry lifetimes and
//! span several `den` modules; rather than migrate them wholesale, this seam
//! exposes owned request/record value types and lets the `den` impl compose the
//! underlying `create_observation` / `create_proposal` / enqueue / mark calls.
//! See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

use crate::{BearProfile, DenError};
use serde_json::Value;
use uuid::Uuid;

/// Closed persisted states accepted by the memory-proposal list filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryProposalStatus {
    Pending,
    Rejected,
    RetainedLocal,
    Deferred,
    Superseded,
    NeedsHumanReview,
}

impl MemoryProposalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Rejected => "rejected",
            Self::RetainedLocal => "retained_local",
            Self::Deferred => "deferred",
            Self::Superseded => "superseded",
            Self::NeedsHumanReview => "needs_human_review",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "rejected" => Some(Self::Rejected),
            "retained_local" => Some(Self::RetainedLocal),
            "deferred" => Some(Self::Deferred),
            "superseded" => Some(Self::Superseded),
            "needs_human_review" => Some(Self::NeedsHumanReview),
            _ => None,
        }
    }
}

/// The observation fields the `observation_write` payload needs.
#[derive(Debug, Clone)]
pub struct ObservationRecord {
    pub bear_id: Uuid,
    pub observation_id: String,
    pub summary: String,
    pub salience: String,
    pub payload_ref: Option<String>,
    pub logical_path: String,
    pub status: String,
    pub proposal_id: Option<Uuid>,
}

/// A validated observation ready to persist + enqueue for curate review.
#[derive(Debug, Clone)]
pub struct ObservationWriteRequest {
    pub bear_id: Uuid,
    pub binding_id: String,
    pub observation_id: String,
    pub summary: String,
    pub salience: String,
    pub payload_ref: Option<String>,
    pub source: Value,
    pub conversation_id: Option<String>,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
}

/// Shared `conversation_events` projection inputs, computed by the executor.
#[derive(Debug, Clone)]
pub struct ProposalProjection {
    pub user_id: i32,
    pub conversation_id: Option<String>,
    pub scope_id: String,
}

/// A validated proposal resolution (`resolve_proposal` / part of `apply_core_update`).
#[derive(Debug, Clone)]
pub struct ResolveProposalRequest {
    pub bear_id: Uuid,
    pub reviewer_profile: BearProfile,
    pub binding_id: String,
    pub proposal_id: Uuid,
    pub status: String,
    pub review_notes: Option<String>,
    pub decision_summary: Option<String>,
    pub projection: ProposalProjection,
}

/// A validated review request (`request_review`).
#[derive(Debug, Clone)]
pub struct RequestReviewRequest {
    pub bear_id: Uuid,
    pub source_profile: BearProfile,
    pub binding_id: Option<String>,
    pub source_paths: Vec<String>,
    pub source_refs: Value,
    pub suggested_action: String,
    pub target_ref: Option<String>,
    pub title: String,
    pub summary: String,
    pub rationale: String,
    pub proposed_content: Option<String>,
    pub proposed_patch: Option<String>,
    pub refs: Value,
    pub sensitivity: String,
    pub requires_human: bool,
    pub projection: ProposalProjection,
}

/// A validated core-update application (`apply_core_update`).
#[derive(Debug, Clone)]
pub struct ApplyCoreUpdateRequest {
    pub bear_id: Uuid,
    pub reviewer_profile: BearProfile,
    pub binding_id: String,
    pub proposal_id: Uuid,
    pub target_path: String,
    pub mode: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub review_notes: Option<String>,
    pub projection: ProposalProjection,
}

/// A validated lifecycle marker for an existing memory record.
#[derive(Debug, Clone)]
pub struct MarkMemoryLifecycleRequest {
    pub bear_id: Uuid,
    pub reviewer_profile: BearProfile,
    pub binding_id: String,
    pub memory_id: String,
    pub status: String,
    pub reason: Option<String>,
}

// Native async fn in trait: workspace-internal, consumed via generic bounds /
// concrete impls only (never `dyn`), so Send flows through monomorphization.
#[allow(async_fn_in_trait)]
pub trait MemoryReviewStore: Send + Sync {
    /// Idempotency check: an existing observation with this id, if any.
    async fn find_observation(
        &self,
        bear_id: Uuid,
        observation_id: &str,
    ) -> Result<Option<ObservationRecord>, DenError>;

    /// Persist the observation and enqueue it for memory-curate review.
    async fn record_observation(
        &self,
        request: ObservationWriteRequest,
    ) -> Result<ObservationRecord, DenError>;

    /// Proposals matching the closed persisted status domain, if filtered.
    async fn list_proposals(
        &self,
        bear_id: Uuid,
        status: Option<MemoryProposalStatus>,
        limit: i64,
    ) -> Result<Value, DenError>;

    /// A single proposal as serialized JSON, if found.
    async fn get_proposal(
        &self,
        bear_id: Uuid,
        proposal_id: Uuid,
    ) -> Result<Option<Value>, DenError>;

    /// Resolve a proposal and project the event; returns the proposal JSON.
    async fn resolve_proposal(&self, request: ResolveProposalRequest) -> Result<Value, DenError>;

    /// Create a review-request proposal and project the event; returns the proposal JSON.
    async fn request_review(&self, request: RequestReviewRequest) -> Result<Value, DenError>;

    /// Promote a reviewed proposal to core, resolve it, and project the event;
    /// returns the full `{bear_id, proposal, core_update}` payload JSON.
    async fn apply_core_update(&self, request: ApplyCoreUpdateRequest) -> Result<Value, DenError>;

    /// Mark an existing memory record's lifecycle status; returns the updated record JSON.
    async fn mark_memory_lifecycle(
        &self,
        request: MarkMemoryLifecycleRequest,
    ) -> Result<Value, DenError>;
}
