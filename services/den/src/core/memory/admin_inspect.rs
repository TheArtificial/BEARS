//! Read-only admin inspection helpers for per-Bear SQLite memory.

use std::path::PathBuf;

use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

use crate::{config::Config, errors::CustomError};

use super::store::{
    list_memory_proposals, memory_sequence_high_water, MemoryRecordRow, MemoryScopeType,
    MemoryStoreManager,
};

#[derive(Debug, Clone, Serialize)]
pub struct BearMemoryAdminStats {
    pub db_path: String,
    pub db_exists: bool,
    pub db_size_bytes: Option<u64>,
    pub record_count: i64,
    pub shared_count: i64,
    pub profile_local_count: i64,
    pub sequence_high_water: i64,
    pub pending_proposals: i64,
    pub pending_observations: i64,
    pub distinct_paths: i64,
}

pub fn bear_sqlite_db_path(config: &Config, bear_id: Uuid) -> PathBuf {
    PathBuf::from(&config.bear_sqlite_data_dir).join(format!("{bear_id}.sqlite"))
}

pub async fn bear_memory_admin_stats(
    manager: &MemoryStoreManager,
    config: &Config,
    bear_id: Uuid,
) -> Result<BearMemoryAdminStats, CustomError> {
    let db_path = bear_sqlite_db_path(config, bear_id);
    let db_path_display = db_path.display().to_string();
    let db_exists = db_path.exists();
    let db_size_bytes = db_exists.then(|| std::fs::metadata(&db_path).ok().map(|m| m.len())).flatten();

    let store = manager.store_for_bear(bear_id).await?;
    let pool = store.pool();

    let record_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memory_records WHERE bear_id = ?",
    )
    .bind(bear_id.to_string())
    .fetch_one(pool)
    .await
    .map_err(|e| CustomError::System(format!("memory record count failed: {e}")))?;

    let shared_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memory_records WHERE bear_id = ? AND scope_type = 'shared'",
    )
    .bind(bear_id.to_string())
    .fetch_one(pool)
    .await
    .map_err(|e| CustomError::System(format!("memory shared count failed: {e}")))?;

    let profile_local_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memory_records WHERE bear_id = ? AND scope_type = 'profile_local'",
    )
    .bind(bear_id.to_string())
    .fetch_one(pool)
    .await
    .map_err(|e| CustomError::System(format!("memory profile_local count failed: {e}")))?;

    let distinct_paths: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT logical_path) FROM memory_records WHERE bear_id = ? AND logical_path IS NOT NULL",
    )
    .bind(bear_id.to_string())
    .fetch_one(pool)
    .await
    .map_err(|e| CustomError::System(format!("memory path count failed: {e}")))?;

    let pending_proposals = list_memory_proposals(&store, Some("pending"), 500)
        .await?
        .len() as i64;

    let pending_observations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memory_observations WHERE bear_id = ? AND status = 'pending_review'",
    )
    .bind(bear_id.to_string())
    .fetch_one(pool)
    .await
    .map_err(|e| CustomError::System(format!("memory observation count failed: {e}")))?;

    let sequence_high_water = memory_sequence_high_water(&store).await?;

    Ok(BearMemoryAdminStats {
        db_path: db_path_display,
        db_exists,
        db_size_bytes,
        record_count,
        shared_count,
        profile_local_count,
        sequence_high_water,
        pending_proposals,
        pending_observations,
        distinct_paths,
    })
}

pub async fn list_all_logical_paths(
    manager: &MemoryStoreManager,
    bear_id: Uuid,
) -> Result<Vec<String>, CustomError> {
    let store = manager.store_for_bear(bear_id).await?;
    let rows = sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT logical_path
        FROM memory_records
        WHERE bear_id = ? AND logical_path IS NOT NULL
        ORDER BY logical_path ASC
        "#,
    )
    .bind(bear_id.to_string())
    .fetch_all(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("list memory paths failed: {e}")))?;
    Ok(rows)
}

pub async fn get_memory_record_by_id(
    manager: &MemoryStoreManager,
    bear_id: Uuid,
    memory_id: &str,
) -> Result<Option<MemoryRecordRow>, CustomError> {
    let store = manager.store_for_bear(bear_id).await?;
    let row = sqlx::query(
        r#"
        SELECT memory_id, sequence_no, scope_type, scope_profile, kind, content_text,
               logical_path, work_surface_ref, metadata_json, created_at
        FROM memory_records
        WHERE bear_id = ? AND memory_id = ?
        "#,
    )
    .bind(bear_id.to_string())
    .bind(memory_id)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("get memory record failed: {e}")))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let metadata_raw: String = row
        .try_get("metadata_json")
        .map_err(|e| CustomError::System(format!("decode metadata_json: {e}")))?;
    let metadata_json = serde_json::from_str(&metadata_raw).unwrap_or(serde_json::json!({}));

    Ok(Some(MemoryRecordRow {
        memory_id: row.try_get("memory_id").map_err(|e| CustomError::System(e.to_string()))?,
        sequence_no: row.try_get("sequence_no").map_err(|e| CustomError::System(e.to_string()))?,
        scope_type: MemoryScopeType::parse(
            &row.try_get::<String, _>("scope_type")
                .map_err(|e| CustomError::System(e.to_string()))?,
        )
        .unwrap_or(MemoryScopeType::ProfileLocal),
        scope_profile: row.try_get("scope_profile").ok(),
        kind: row.try_get("kind").map_err(|e| CustomError::System(e.to_string()))?,
        content_text: row
            .try_get("content_text")
            .map_err(|e| CustomError::System(e.to_string()))?,
        logical_path: row.try_get("logical_path").ok(),
        work_surface_ref: row.try_get("work_surface_ref").ok(),
        metadata_json,
        created_at: row.try_get("created_at").map_err(|e| CustomError::System(e.to_string()))?,
    }))
}

pub async fn list_recent_memory_records(
    manager: &MemoryStoreManager,
    bear_id: Uuid,
    limit: i64,
) -> Result<Vec<MemoryRecordRow>, CustomError> {
    let store = manager.store_for_bear(bear_id).await?;
    let rows = sqlx::query(
        r#"
        SELECT memory_id, sequence_no, scope_type, scope_profile, kind, content_text,
               logical_path, work_surface_ref, metadata_json, created_at
        FROM memory_records
        WHERE bear_id = ?
        ORDER BY sequence_no DESC
        LIMIT ?
        "#,
    )
    .bind(bear_id.to_string())
    .bind(limit.clamp(1, 50))
    .fetch_all(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("list recent memory records failed: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let metadata_raw: String = row.try_get("metadata_json")?;
            let metadata_json =
                serde_json::from_str(&metadata_raw).unwrap_or(serde_json::json!({}));
            Ok(MemoryRecordRow {
                memory_id: row.try_get("memory_id")?,
                sequence_no: row.try_get("sequence_no")?,
                scope_type: MemoryScopeType::parse(
                    &row.try_get::<String, _>("scope_type")?,
                )
                .unwrap_or(MemoryScopeType::ProfileLocal),
                scope_profile: row.try_get("scope_profile").ok(),
                kind: row.try_get("kind")?,
                content_text: row.try_get("content_text")?,
                logical_path: row.try_get("logical_path").ok(),
                work_surface_ref: row.try_get("work_surface_ref").ok(),
                metadata_json,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|e| CustomError::System(format!("decode memory records: {e}")))
}
