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

pub mod chunking;
pub mod indexer;
pub mod policy;
pub mod qdrant;
pub mod reconcile;
pub mod registry;

pub use indexer::{IndexOutcome, PassageEmbedder, RecallIndexer};
pub use policy::IndexRequest;
pub use qdrant::{collection_name, QdrantPoint, QdrantRecall};
pub use reconcile::{reconcile_bear, ReconcileOutcome};

#[cfg(any(test, feature = "test-util"))]
pub use indexer::DeterministicEmbedder;
