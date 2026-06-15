//! Whole-Bear reconcile (ADR-0038 §6): bring the derived recall index in line with the
//! canonical SQLite heads. Indexes every indexable head record and removes passages for
//! memory ids that are no longer heads (supersede/delete). Idempotent and bounded per Bear.

use std::collections::HashSet;

use sqlx::PgPool;

use den_core::DenError;

use crate::memory::store::BearMemoryStore;

use super::indexer::{PassageEmbedder, RecallIndexer};
use super::policy::{is_indexable, IndexRequest};
use super::qdrant::QdrantRecall;
use super::registry;

/// Aggregate result of a reconcile pass (diagnostics + tests).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileOutcome {
    pub indexed_records: usize,
    pub embedded_chunks: usize,
    pub reused_chunks: usize,
    pub removed_records: usize,
    pub removed_points: usize,
}

/// The current indexable head record per logical path for a Bear (latest, normal-visibility,
/// not superseded). Non-indexable kinds/scopes are filtered out per [`is_indexable`].
pub async fn list_indexable_heads(store: &BearMemoryStore) -> Result<Vec<IndexRequest>, DenError> {
    let bear_id = store.bear_id();
    type HeadRow = (
        String,         // memory_id
        String,         // scope_type
        Option<String>, // scope_profile
        String,         // kind
        String,         // visibility
        Option<String>, // logical_path
        Option<String>, // work_surface_ref
        String,         // content_text
    );
    let rows = sqlx::query_as::<_, HeadRow>(
        r"
        SELECT m.memory_id, m.scope_type, m.scope_profile, m.kind, m.visibility,
               m.logical_path, m.work_surface_ref, m.content_text
        FROM memory_records m
        WHERE m.bear_id = ?
          AND m.visibility = 'normal'
          AND m.logical_path IS NOT NULL
          AND NOT EXISTS (
              SELECT 1 FROM memory_records n
              WHERE n.bear_id = m.bear_id AND n.supersedes_memory_id = m.memory_id
          )
          AND m.sequence_no = (
              SELECT MAX(h.sequence_no) FROM memory_records h
              WHERE h.bear_id = m.bear_id
                AND h.logical_path = m.logical_path
                AND h.visibility = 'normal'
          )
        ORDER BY m.logical_path
        ",
    )
    .bind(bear_id.to_string())
    .fetch_all(store.pool())
    .await
    .map_err(|e| DenError::System(format!("list indexable heads: {e}")))?;

    Ok(rows
        .into_iter()
        .filter_map(
            |(
                memory_id,
                scope_type,
                scope_profile,
                kind,
                visibility,
                logical_path,
                work_surface_ref,
                content_text,
            )| {
                if !is_indexable(&scope_type, &kind, &visibility) {
                    return None;
                }
                Some(IndexRequest {
                    bear_id,
                    memory_id,
                    logical_path,
                    scope_type,
                    scope_profile,
                    work_surface_ref,
                    kind,
                    visibility,
                    content_text,
                })
            },
        )
        .collect())
}

/// Reconcile a Bear's recall index against its canonical heads.
pub async fn reconcile_bear<E: PassageEmbedder>(
    pg: &PgPool,
    qdrant: &QdrantRecall,
    embedder: &E,
    store: &BearMemoryStore,
    embedding_standard: &str,
) -> Result<ReconcileOutcome, DenError> {
    let bear_id = store.bear_id();
    let heads = list_indexable_heads(store).await?;
    let indexer = RecallIndexer::new(pg, qdrant, embedder, embedding_standard.to_string());

    let mut outcome = ReconcileOutcome::default();
    let mut head_ids: HashSet<String> = HashSet::new();
    for req in &heads {
        head_ids.insert(req.memory_id.clone());
        let o = indexer.index_record(req).await?;
        outcome.indexed_records += 1;
        outcome.embedded_chunks += o.embedded_chunks;
        outcome.reused_chunks += o.reused_chunks;
        outcome.removed_points += o.removed_points;
    }

    let indexed_ids = registry::list_indexed_memory_ids(pg, bear_id, embedding_standard).await?;
    for mid in indexed_ids {
        if !head_ids.contains(&mid) {
            let removed = indexer.remove_record(bear_id, &mid).await?;
            outcome.removed_records += 1;
            outcome.removed_points += removed;
        }
    }
    Ok(outcome)
}
