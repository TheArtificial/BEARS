//! The `MemoryReviewStore` capability seam: reflection/curation review surface
//! (observations + proposals + enqueue + projections).
//!
//! The granular den proposal/observation row + param types carry lifetimes and
//! span several `den` modules; rather than migrate them wholesale, this seam
//! exposes owned request/record value types and lets the `den` impl compose the
//! underlying `create_observation` / `create_proposal` / enqueue / mark calls.
//! See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

use async_trait::async_trait;
use den_core::DenError;
use serde_json::Value;
use uuid::Uuid;

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

#[async_trait]
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
}
