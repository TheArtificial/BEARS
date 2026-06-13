use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

use super::logical_path::{LogicalMemoryPath, MemoryScopeType};

#[derive(Debug, Clone, Serialize)]
pub struct MemoryRecordRow {
    pub memory_id: String,
    pub sequence_no: i64,
    pub scope_type: MemoryScopeType,
    pub scope_profile: Option<String>,
    pub kind: String,
    pub content_text: String,
    pub logical_path: Option<String>,
    pub work_surface_ref: Option<String>,
    pub metadata_json: Value,
    pub created_at: String,
}

pub struct BearMemoryStore {
    bear_id: Uuid,
    pool: SqlitePool,
}

impl BearMemoryStore {
    pub fn new(bear_id: Uuid, pool: SqlitePool) -> Self {
        Self { bear_id, pool }
    }

    pub fn bear_id(&self) -> Uuid {
        self.bear_id
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn next_sequence(&self) -> Result<i64, DenError> {
        sqlx::query("UPDATE bear_sequence SET next_sequence = next_sequence + 1 WHERE id = 1")
            .execute(&self.pool)
            .await
            .map_err(|e| DenError::System(format!("bear sequence bump failed: {e}")))?;
        let row = sqlx::query_scalar::<_, i64>(
            "SELECT next_sequence - 1 FROM bear_sequence WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DenError::System(format!("bear sequence alloc failed: {e}")))?;
        Ok(row)
    }

    pub async fn append_record(
        &self,
        logical: &LogicalMemoryPath,
        kind: &str,
        author_profile: &str,
        author_agent_id: Option<&str>,
        content_text: &str,
        metadata_json: &Value,
        visibility: &str,
    ) -> Result<MemoryRecordRow, DenError> {
        let memory_id = Uuid::new_v4().to_string();
        let sequence_no = self.next_sequence().await?;
        let created_at = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| DenError::System(format!("timestamp format failed: {e}")))?;
        let logical_path = logical.to_logical_path();
        sqlx::query(
            r#"
            INSERT INTO memory_records (
                memory_id, bear_id, sequence_no, scope_type, scope_profile, kind, entity_ref,
                author_profile, author_agent_id, created_at, content_text, metadata_json,
                visibility, logical_path, work_surface_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&memory_id)
        .bind(self.bear_id.to_string())
        .bind(sequence_no)
        .bind(logical.scope_type.as_str())
        .bind(&logical.scope_profile)
        .bind(kind)
        .bind(&logical.entity_ref)
        .bind(author_profile)
        .bind(author_agent_id)
        .bind(&created_at)
        .bind(content_text)
        .bind(metadata_json.to_string())
        .bind(visibility)
        .bind(&logical_path)
        .bind(&logical.work_surface_ref)
        .execute(&self.pool)
        .await
        .map_err(|e| DenError::System(format!("append memory_record failed: {e}")))?;
        Ok(MemoryRecordRow {
            memory_id,
            sequence_no,
            scope_type: logical.scope_type,
            scope_profile: logical.scope_profile.clone(),
            kind: kind.to_string(),
            content_text: content_text.to_string(),
            logical_path: Some(logical_path),
            work_surface_ref: logical.work_surface_ref.clone(),
            metadata_json: metadata_json.clone(),
            created_at,
        })
    }
}

pub async fn append_memory_record(
    store: &BearMemoryStore,
    logical: &LogicalMemoryPath,
    kind: &str,
    author_profile: &str,
    author_agent_id: Option<&str>,
    content_text: &str,
    metadata_json: &Value,
) -> Result<MemoryRecordRow, DenError> {
    store
        .append_record(
            logical,
            kind,
            author_profile,
            author_agent_id,
            content_text,
            metadata_json,
            "normal",
        )
        .await
}

pub async fn memory_sequence_high_water(store: &BearMemoryStore) -> Result<i64, DenError> {
    let row = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(sequence_no) FROM memory_records WHERE bear_id = ?",
    )
    .bind(store.bear_id.to_string())
    .fetch_one(store.pool())
    .await
    .map_err(|e| DenError::System(format!("memory sequence high water failed: {e}")))?;
    Ok(row.unwrap_or(0))
}

pub async fn head_record_for_logical_path(
    store: &BearMemoryStore,
    logical_path: &str,
) -> Result<Option<MemoryRecordRow>, DenError> {
    let row = sqlx::query_as::<_, MemoryRecordSqlRow>(
        r#"
        SELECT memory_id, sequence_no, scope_type, scope_profile, kind, content_text,
               logical_path, work_surface_ref, metadata_json, created_at
        FROM memory_records
        WHERE bear_id = ? AND logical_path = ? AND visibility = 'normal'
          AND NOT EXISTS (
            SELECT 1 FROM memory_records newer
            WHERE newer.bear_id = memory_records.bear_id
              AND newer.supersedes_memory_id = memory_records.memory_id
          )
        ORDER BY sequence_no DESC
        LIMIT 1
        "#,
    )
    .bind(store.bear_id.to_string())
    .bind(logical_path)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| DenError::System(format!("head memory_record lookup failed: {e}")))?;
    Ok(row.map(MemoryRecordSqlRow::into_row))
}

pub async fn has_work_surface_canonical_anchor(
    store: &BearMemoryStore,
    slug: &str,
) -> Result<bool, DenError> {
    for path in [
        format!("core/work_surfaces/{slug}/index.md"),
        format!("core/work_surfaces/{slug}/overview.md"),
    ] {
        if head_record_for_logical_path(store, &path).await?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn list_profile_local_head_records(
    store: &BearMemoryStore,
    profile: &str,
    work_surface_ref: Option<&str>,
    limit: i64,
) -> Result<Vec<MemoryRecordRow>, DenError> {
    let rows = if let Some(surface) = work_surface_ref {
        sqlx::query_as::<_, MemoryRecordSqlRow>(
            r#"
            SELECT memory_id, sequence_no, scope_type, scope_profile, kind, content_text,
                   logical_path, work_surface_ref, metadata_json, created_at
            FROM memory_records
            WHERE bear_id = ? AND scope_type = 'profile_local' AND scope_profile = ?
              AND visibility = 'normal' AND work_surface_ref = ?
              AND NOT EXISTS (
                SELECT 1 FROM memory_records newer
                WHERE newer.bear_id = memory_records.bear_id
                  AND newer.supersedes_memory_id = memory_records.memory_id
              )
            ORDER BY sequence_no DESC
            LIMIT ?
            "#,
        )
        .bind(store.bear_id.to_string())
        .bind(profile)
        .bind(surface)
        .bind(limit)
        .fetch_all(store.pool())
        .await
    } else {
        sqlx::query_as::<_, MemoryRecordSqlRow>(
            r#"
            SELECT memory_id, sequence_no, scope_type, scope_profile, kind, content_text,
                   logical_path, work_surface_ref, metadata_json, created_at
            FROM memory_records
            WHERE bear_id = ? AND scope_type = 'profile_local' AND scope_profile = ?
              AND visibility = 'normal' AND work_surface_ref IS NULL
              AND NOT EXISTS (
                SELECT 1 FROM memory_records newer
                WHERE newer.bear_id = memory_records.bear_id
                  AND newer.supersedes_memory_id = memory_records.memory_id
              )
            ORDER BY sequence_no DESC
            LIMIT ?
            "#,
        )
        .bind(store.bear_id.to_string())
        .bind(profile)
        .bind(limit)
        .fetch_all(store.pool())
        .await
    }
    .map_err(|e| DenError::System(format!("list profile_local memory_records failed: {e}")))?;
    Ok(rows.into_iter().map(MemoryRecordSqlRow::into_row).collect())
}

pub async fn list_records_for_logical_path(
    store: &BearMemoryStore,
    logical_path: &str,
    limit: i64,
) -> Result<Vec<MemoryRecordRow>, DenError> {
    let rows = sqlx::query_as::<_, MemoryRecordSqlRow>(
        r#"
        SELECT memory_id, sequence_no, scope_type, scope_profile, kind, content_text,
               logical_path, work_surface_ref, metadata_json, created_at
        FROM memory_records
        WHERE bear_id = ? AND logical_path = ?
        ORDER BY sequence_no DESC
        LIMIT ?
        "#,
    )
    .bind(store.bear_id.to_string())
    .bind(logical_path)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(|e| DenError::System(format!("list memory_records failed: {e}")))?;
    Ok(rows.into_iter().map(MemoryRecordSqlRow::into_row).collect())
}

#[derive(sqlx::FromRow)]
struct MemoryRecordSqlRow {
    memory_id: String,
    sequence_no: i64,
    scope_type: String,
    scope_profile: Option<String>,
    kind: String,
    content_text: String,
    logical_path: Option<String>,
    work_surface_ref: Option<String>,
    metadata_json: String,
    created_at: String,
}

impl MemoryRecordSqlRow {
    fn into_row(self) -> MemoryRecordRow {
        MemoryRecordRow {
            memory_id: self.memory_id,
            sequence_no: self.sequence_no,
            scope_type: MemoryScopeType::parse(&self.scope_type)
                .unwrap_or(MemoryScopeType::ProfileLocal),
            scope_profile: self.scope_profile,
            kind: self.kind,
            content_text: self.content_text,
            logical_path: self.logical_path,
            work_surface_ref: self.work_surface_ref,
            metadata_json: serde_json::from_str(&self.metadata_json)
                .unwrap_or_else(|_| Value::Object(Default::default())),
            created_at: self.created_at,
        }
    }
}
