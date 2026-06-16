//! Derived recall index (ADR-0038): a Qdrant-backed semantic index over canonical Bear
//! memory. Vectors are **derived** — SQLite is the source of truth and Qdrant may be absent
//! (callers degrade to keyword fallback).
//!
//! - [`qdrant`] — minimal Qdrant REST client (readiness, collection bootstrap, points).
//! - [`chunking`] — passage chunking + content hashing.
//! - [`policy`] — indexing policy + payload shaping.
//! - [`registry`] — Postgres `recall_passages` metadata.
//! - [`indexer`] — orchestration (chunk → embed → upsert → register).
//! - [`reconcile`] — whole-Bear reconcile against canonical heads.
//! - [`query`] — recall query for the turn assembler (embed → search → render).
//! - [`temporal`] — time-expression parsing for the temporal recall leg (Phase 3.5).

pub mod chunking;
pub mod indexer;
pub mod policy;
pub mod qdrant;
pub mod query;
pub mod reconcile;
pub mod registry;
pub mod temporal;

pub use indexer::{IndexOutcome, PassageEmbedder, RecallIndexer};
pub use policy::IndexRequest;
pub use qdrant::{collection_name, QdrantPoint, QdrantRecall, RecallHit};
pub use query::{
    graph_expand_hits, hybrid_memory_search, recall_for_turn, recall_for_turn_scoped,
    render_recall_block, search_bear_memory_for_entities, search_bear_memory_for_role,
    semantic_search_for_bear, RecallProjection, RecalledPassage,
};
pub use reconcile::{reconcile_bear, ReconcileOutcome};
pub use temporal::{parse_time_expression, TemporalQuery};

#[cfg(any(test, feature = "test-util"))]
pub use indexer::DeterministicEmbedder;
