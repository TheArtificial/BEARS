//! `recall_passages` registry access (ADR-0038 §3): Postgres metadata for the derived
//! recall index. Enables idempotent upsert, content-hash dedup, and delete-on-supersede.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use den_core::DenError;

fn db_err(operation: &'static str) -> impl FnOnce(sqlx::Error) -> DenError {
    move |err| DenError::System(format!("recall_passages {operation}: {err}"))
}

/// A live (non-deleted) registry row for an indexed chunk.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ExistingPassage {
    pub chunk_index: i32,
    pub content_hash: String,
    pub point_id: String,
}

/// Live content hash for one indexed memory chunk.
#[derive(Debug, Clone)]
struct LiveChunkHashRow {
    memory_id: String,
    chunk_index: i32,
    content_hash: String,
}

/// List live passages for a memory record, for dedup + stale-chunk pruning.
pub async fn list_passages(
    pool: &PgPool,
    bear_id: Uuid,
    memory_id: &str,
    embedding_standard: &str,
) -> Result<Vec<ExistingPassage>, DenError> {
    let rows = sqlx::query_as!(
        ExistingPassage,
        r#"
        SELECT chunk_index AS "chunk_index!", content_hash AS "content_hash!",
               point_id AS "point_id!"
        FROM recall_passages
        WHERE bear_id = $1 AND memory_id = $2 AND embedding_standard = $3
          AND deleted_at IS NULL
        ORDER BY chunk_index
        "#,
        bear_id,
        memory_id,
        embedding_standard,
    )
    .fetch_all(pool)
    .await
    .map_err(db_err("list"))?;

    Ok(rows)
}

/// Distinct memory ids that currently have live passages for a bear (used by reconcile to
/// find records that are no longer canonical heads and should be removed).
pub async fn list_indexed_memory_ids(
    pool: &PgPool,
    bear_id: Uuid,
    embedding_standard: &str,
) -> Result<Vec<String>, DenError> {
    let rows = sqlx::query_scalar!(
        r#"
        SELECT DISTINCT memory_id AS "memory_id!"
        FROM recall_passages
        WHERE bear_id = $1 AND embedding_standard = $2 AND deleted_at IS NULL
        "#,
        bear_id,
        embedding_standard,
    )
    .fetch_all(pool)
    .await
    .map_err(db_err("list memory ids"))?;
    Ok(rows)
}

/// Live (non-deleted) chunk content hashes for every indexed memory record of a Bear,
/// keyed `memory_id → chunk_index → content_hash`. Feeds the recall consistency
/// watermark's canonical-vs-registry comparison (ADR-0038 §8).
pub async fn live_chunk_hashes_by_memory(
    pool: &PgPool,
    bear_id: Uuid,
    embedding_standard: &str,
) -> Result<HashMap<String, HashMap<i32, String>>, DenError> {
    let rows = sqlx::query_as!(
        LiveChunkHashRow,
        r#"
        SELECT memory_id AS "memory_id!", chunk_index AS "chunk_index!",
               content_hash AS "content_hash!"
        FROM recall_passages
        WHERE bear_id = $1 AND embedding_standard = $2 AND deleted_at IS NULL
        "#,
        bear_id,
        embedding_standard,
    )
    .fetch_all(pool)
    .await
    .map_err(db_err("list chunk hashes"))?;

    let mut by_memory: HashMap<String, HashMap<i32, String>> = HashMap::new();
    for row in rows {
        by_memory
            .entry(row.memory_id)
            .or_default()
            .insert(row.chunk_index, row.content_hash);
    }
    Ok(by_memory)
}

/// Per-Bear recall coverage stats: live passage (chunk) count and the number of distinct
/// memory records that have at least one live passage. Used by the memory admin dashboard.
pub async fn passage_stats(
    pool: &PgPool,
    bear_id: Uuid,
    embedding_standard: &str,
) -> Result<(i64, i64), DenError> {
    let row = sqlx::query!(
        r#"
        SELECT COUNT(*)::bigint AS "passages!",
               COUNT(DISTINCT memory_id)::bigint AS "memories!"
        FROM recall_passages
        WHERE bear_id = $1 AND embedding_standard = $2 AND deleted_at IS NULL
        "#,
        bear_id,
        embedding_standard,
    )
    .fetch_one(pool)
    .await
    .map_err(db_err("stats"))?;
    Ok((row.passages, row.memories))
}

pub struct NewPassage<'a> {
    pub bear_id: Uuid,
    pub memory_id: &'a str,
    pub logical_path: Option<&'a str>,
    pub chunk_index: i32,
    pub content_hash: &'a str,
    pub embedding_standard: &'a str,
    pub source_class: &'a str,
    pub point_id: &'a str,
}

/// Insert or refresh a passage registry row (idempotent on the chunk identity).
pub async fn upsert_passage(pool: &PgPool, passage: NewPassage<'_>) -> Result<(), DenError> {
    sqlx::query!(
        r#"
        INSERT INTO recall_passages (
            bear_id, memory_id, logical_path, chunk_index, content_hash,
            embedding_standard, source_class, point_id, indexed_at, deleted_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(), NULL)
        ON CONFLICT (bear_id, memory_id, chunk_index, embedding_standard)
        DO UPDATE SET
            logical_path = EXCLUDED.logical_path,
            content_hash = EXCLUDED.content_hash,
            source_class = EXCLUDED.source_class,
            point_id = EXCLUDED.point_id,
            indexed_at = NOW(),
            deleted_at = NULL
        "#,
        passage.bear_id,
        passage.memory_id,
        passage.logical_path,
        passage.chunk_index,
        passage.content_hash,
        passage.embedding_standard,
        passage.source_class,
        passage.point_id,
    )
    .execute(pool)
    .await
    .map_err(db_err("upsert"))?;
    Ok(())
}

/// Soft-delete all live passages for a memory record; returns their Qdrant point ids so the
/// caller can delete the vectors.
pub async fn delete_passages_for_memory(
    pool: &PgPool,
    bear_id: Uuid,
    memory_id: &str,
    embedding_standard: &str,
) -> Result<Vec<String>, DenError> {
    let rows = sqlx::query_scalar!(
        r#"
        UPDATE recall_passages
        SET deleted_at = NOW()
        WHERE bear_id = $1 AND memory_id = $2 AND embedding_standard = $3
          AND deleted_at IS NULL
        RETURNING point_id AS "point_id!"
        "#,
        bear_id,
        memory_id,
        embedding_standard,
    )
    .fetch_all(pool)
    .await
    .map_err(db_err("delete (memory)"))?;

    Ok(rows)
}

/// Soft-delete live passages for a memory record at or above `min_chunk_index` (stale-chunk
/// pruning when a record's content shrinks); returns their Qdrant point ids.
pub async fn delete_passages_for_chunks_ge(
    pool: &PgPool,
    bear_id: Uuid,
    memory_id: &str,
    embedding_standard: &str,
    min_chunk_index: i32,
) -> Result<Vec<String>, DenError> {
    let rows = sqlx::query_scalar!(
        r#"
        UPDATE recall_passages
        SET deleted_at = NOW()
        WHERE bear_id = $1 AND memory_id = $2 AND embedding_standard = $3
          AND chunk_index >= $4 AND deleted_at IS NULL
        RETURNING point_id AS "point_id!"
        "#,
        bear_id,
        memory_id,
        embedding_standard,
        min_chunk_index,
    )
    .fetch_all(pool)
    .await
    .map_err(db_err("delete (chunks)"))?;

    Ok(rows)
}
