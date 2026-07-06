//! `recall_passages` registry access (ADR-0038 §3): Postgres metadata for the derived
//! recall index. Enables idempotent upsert, content-hash dedup, and delete-on-supersede.

use sqlx::{PgPool, Row};
use uuid::Uuid;

use den_core::DenError;

/// A live (non-deleted) registry row for an indexed chunk.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ExistingPassage {
    pub chunk_index: i32,
    pub content_hash: String,
    pub point_id: String,
}

/// List live passages for a memory record, for dedup + stale-chunk pruning.
pub async fn list_passages(
    pool: &PgPool,
    bear_id: Uuid,
    memory_id: &str,
    embedding_standard: &str,
) -> Result<Vec<ExistingPassage>, DenError> {
    let rows = sqlx::query_as::<_, ExistingPassage>(
        r"
        SELECT chunk_index, content_hash, point_id
        FROM recall_passages
        WHERE bear_id = $1 AND memory_id = $2 AND embedding_standard = $3
          AND deleted_at IS NULL
        ORDER BY chunk_index
        ",
    )
    .bind(bear_id)
    .bind(memory_id)
    .bind(embedding_standard)
    .fetch_all(pool)
    .await
    .map_err(|e| DenError::System(format!("recall_passages list: {e}")))?;

    Ok(rows)
}

/// Distinct memory ids that currently have live passages for a bear (used by reconcile to
/// find records that are no longer canonical heads and should be removed).
pub async fn list_indexed_memory_ids(
    pool: &PgPool,
    bear_id: Uuid,
    embedding_standard: &str,
) -> Result<Vec<String>, DenError> {
    let rows = sqlx::query_scalar::<_, String>(
        r"
        SELECT DISTINCT memory_id
        FROM recall_passages
        WHERE bear_id = $1 AND embedding_standard = $2 AND deleted_at IS NULL
        ",
    )
    .bind(bear_id)
    .bind(embedding_standard)
    .fetch_all(pool)
    .await
    .map_err(|e| DenError::System(format!("recall_passages list memory ids: {e}")))?;
    Ok(rows)
}

/// Per-Bear recall coverage stats: live passage (chunk) count and the number of distinct
/// memory records that have at least one live passage. Used by the memory admin dashboard.
pub async fn passage_stats(
    pool: &PgPool,
    bear_id: Uuid,
    embedding_standard: &str,
) -> Result<(i64, i64), DenError> {
    let row = sqlx::query(
        r"
        SELECT COUNT(*) AS passages, COUNT(DISTINCT memory_id) AS memories
        FROM recall_passages
        WHERE bear_id = $1 AND embedding_standard = $2 AND deleted_at IS NULL
        ",
    )
    .bind(bear_id)
    .bind(embedding_standard)
    .fetch_one(pool)
    .await
    .map_err(|e| DenError::System(format!("recall_passages stats: {e}")))?;
    let passages: i64 = row
        .try_get("passages")
        .map_err(|e| DenError::System(e.to_string()))?;
    let memories: i64 = row
        .try_get("memories")
        .map_err(|e| DenError::System(e.to_string()))?;
    Ok((passages, memories))
}

/// Insert or refresh a passage registry row (idempotent on the chunk identity).
#[allow(clippy::too_many_arguments)]
pub async fn upsert_passage(
    pool: &PgPool,
    bear_id: Uuid,
    memory_id: &str,
    logical_path: Option<&str>,
    chunk_index: i32,
    content_hash: &str,
    embedding_standard: &str,
    source_class: &str,
    point_id: &str,
) -> Result<(), DenError> {
    sqlx::query(
        r"
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
        ",
    )
    .bind(bear_id)
    .bind(memory_id)
    .bind(logical_path)
    .bind(chunk_index)
    .bind(content_hash)
    .bind(embedding_standard)
    .bind(source_class)
    .bind(point_id)
    .execute(pool)
    .await
    .map_err(|e| DenError::System(format!("recall_passages upsert: {e}")))?;
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
    let rows = sqlx::query(
        r"
        UPDATE recall_passages
        SET deleted_at = NOW()
        WHERE bear_id = $1 AND memory_id = $2 AND embedding_standard = $3
          AND deleted_at IS NULL
        RETURNING point_id
        ",
    )
    .bind(bear_id)
    .bind(memory_id)
    .bind(embedding_standard)
    .fetch_all(pool)
    .await
    .map_err(|e| DenError::System(format!("recall_passages delete (memory): {e}")))?;

    Ok(rows
        .into_iter()
        .map(|r| r.get::<String, _>("point_id"))
        .collect())
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
    let rows = sqlx::query(
        r"
        UPDATE recall_passages
        SET deleted_at = NOW()
        WHERE bear_id = $1 AND memory_id = $2 AND embedding_standard = $3
          AND chunk_index >= $4 AND deleted_at IS NULL
        RETURNING point_id
        ",
    )
    .bind(bear_id)
    .bind(memory_id)
    .bind(embedding_standard)
    .bind(min_chunk_index)
    .fetch_all(pool)
    .await
    .map_err(|e| DenError::System(format!("recall_passages delete (chunks): {e}")))?;

    Ok(rows
        .into_iter()
        .map(|r| r.get::<String, _>("point_id"))
        .collect())
}
