//! Whole-Bear reconcile (ADR-0038 §6): bring the derived recall index in line with the
//! canonical SQLite heads. Indexes every indexable head record and removes passages for
//! memory ids that are no longer heads (supersede/delete). Idempotent and bounded per Bear.

use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

use den_core::{config::Config, DenError};
use den_llm::EmbeddingClient;

use den_memory::MemoryStoreManager;
use den_memory::{relations, BearMemoryStore};

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
        String,         // salience
        Option<String>, // logical_path
        Option<String>, // work_surface_ref
        String,         // content_text
    );
    let rows = sqlx::query_as::<_, HeadRow>(
        r"
        SELECT m.memory_id, m.scope_type, m.scope_profile, m.kind, m.visibility,
               m.salience, m.logical_path, m.work_surface_ref, m.content_text
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

    // One bulk pass for resolved descriptive entities, denormalized into each passage payload.
    let entity_ids_by_source = relations::descriptive_entity_ids_by_source(store).await?;

    Ok(rows
        .into_iter()
        .filter_map(
            |(
                memory_id,
                scope_type,
                scope_profile,
                kind,
                visibility,
                salience,
                logical_path,
                work_surface_ref,
                content_text,
            )| {
                if !is_indexable(&scope_type, &kind, &visibility) {
                    return None;
                }
                let entity_ids = entity_ids_by_source
                    .get(&memory_id)
                    .cloned()
                    .unwrap_or_default();
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
                    salience,
                    entity_ids,
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

/// Operator-facing synchronous reindex (ADR-0038 Phase 5 tooling): reconcile one Bear's recall
/// index immediately with the live embedding client, bypassing the `recall_index` queue. Used by
/// the `den reindex` CLI. Errors if recall is disabled (`QDRANT_URL` unset).
pub async fn reindex_bear_now(
    pg: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
) -> Result<ReconcileOutcome, DenError> {
    let qdrant = QdrantRecall::from_config(config)
        .ok_or_else(|| DenError::System("recall disabled (QDRANT_URL unset)".to_string()))?;
    let store = stores.store_for_bear(bear_id).await?;
    let embedder = EmbeddingClient::new(config);
    reconcile_bear(pg, &qdrant, &embedder, &store, &config.embedding_standard).await
}
