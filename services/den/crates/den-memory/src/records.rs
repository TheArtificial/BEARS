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
            r"
            INSERT INTO memory_records (
                memory_id, bear_id, sequence_no, scope_type, scope_profile, kind,
                author_profile, author_agent_id, created_at, content_text, metadata_json,
                visibility, logical_path, work_surface_ref, valid_from
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(&memory_id)
        .bind(self.bear_id.to_string())
        .bind(sequence_no)
        .bind(logical.scope_type.as_str())
        .bind(&logical.scope_profile)
        .bind(kind)
        .bind(author_profile)
        .bind(author_agent_id)
        .bind(&created_at)
        .bind(content_text)
        .bind(metadata_json.to_string())
        .bind(visibility)
        .bind(&logical_path)
        .bind(&logical.work_surface_ref)
        .bind(&created_at)
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
        r"
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
        ",
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
            r"
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
            ",
        )
        .bind(store.bear_id.to_string())
        .bind(profile)
        .bind(surface)
        .bind(limit)
        .fetch_all(store.pool())
        .await
    } else {
        sqlx::query_as::<_, MemoryRecordSqlRow>(
            r"
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
            ",
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
        r"
        SELECT memory_id, sequence_no, scope_type, scope_profile, kind, content_text,
               logical_path, work_surface_ref, metadata_json, created_at
        FROM memory_records
        WHERE bear_id = ? AND logical_path = ?
        ORDER BY sequence_no DESC
        LIMIT ?
        ",
    )
    .bind(store.bear_id.to_string())
    .bind(logical_path)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(|e| DenError::System(format!("list memory_records failed: {e}")))?;
    Ok(rows.into_iter().map(MemoryRecordSqlRow::into_row).collect())
}

/// Minimal record fields for rendering a recall hit (bounded-graph leg). Cheaper than a full
/// [`MemoryRecordRow`] and carries no metadata/sequence.
#[derive(Debug, Clone, Serialize)]
pub struct RecallRecordMin {
    pub memory_id: String,
    pub logical_path: Option<String>,
    pub kind: String,
    pub content_text: String,
}

/// Fetch minimal fields for a set of memory ids, **role-scoped** to memory visible to `role`
/// (shared ∨ own profile-local) at normal visibility. Order is unspecified and role-hidden ids are
/// dropped; the caller re-associates by id. Used by the bounded-graph recall leg.
pub async fn fetch_records_min(
    store: &BearMemoryStore,
    memory_ids: &[String],
    role: &str,
) -> Result<Vec<RecallRecordMin>, DenError> {
    if memory_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; memory_ids.len()].join(",");
    let sql = format!(
        "SELECT memory_id, logical_path, kind, content_text FROM memory_records \
         WHERE bear_id = ? AND visibility = 'normal' \
           AND (scope_type = 'shared' OR scope_profile = ?) \
           AND memory_id IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, (String, Option<String>, String, String)>(&sql)
        .bind(store.bear_id().to_string())
        .bind(role);
    for id in memory_ids {
        query = query.bind(id);
    }
    let rows = query
        .fetch_all(store.pool())
        .await
        .map_err(|e| DenError::System(format!("fetch records min failed: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|(memory_id, logical_path, kind, content_text)| RecallRecordMin {
            memory_id,
            logical_path,
            kind,
            content_text,
        })
        .collect())
}

fn parse_rfc3339(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

/// Effective event time `COALESCE(valid_from, created_at)` parsed to UTC instants, keyed by memory
/// id (Phase 3.5 temporal leg). Unparseable timestamps are skipped. Empty in ⇒ empty out.
pub async fn effective_time_by_ids(
    store: &BearMemoryStore,
    memory_ids: &[String],
) -> Result<std::collections::HashMap<String, OffsetDateTime>, DenError> {
    if memory_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let placeholders = vec!["?"; memory_ids.len()].join(",");
    let sql = format!(
        "SELECT memory_id, COALESCE(valid_from, created_at) FROM memory_records \
         WHERE bear_id = ? AND memory_id IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, (String, String)>(&sql).bind(store.bear_id().to_string());
    for id in memory_ids {
        query = query.bind(id);
    }
    let rows = query
        .fetch_all(store.pool())
        .await
        .map_err(|e| DenError::System(format!("effective time lookup failed: {e}")))?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, ts)| parse_rfc3339(&ts).map(|t| (id, t)))
        .collect())
}

/// For each of `memory_ids`, the effective times of records that supersede it (records whose
/// `supersedes_memory_id` points at it). Used to exclude records already superseded as of a
/// point-in-time query (Phase 3.5).
pub async fn superseder_times_by_superseded(
    store: &BearMemoryStore,
    memory_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<OffsetDateTime>>, DenError> {
    if memory_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let placeholders = vec!["?"; memory_ids.len()].join(",");
    let sql = format!(
        "SELECT supersedes_memory_id, COALESCE(valid_from, created_at) FROM memory_records \
         WHERE bear_id = ? AND supersedes_memory_id IN ({placeholders})"
    );
    let mut query =
        sqlx::query_as::<_, (Option<String>, String)>(&sql).bind(store.bear_id().to_string());
    for id in memory_ids {
        query = query.bind(id);
    }
    let rows = query
        .fetch_all(store.pool())
        .await
        .map_err(|e| DenError::System(format!("superseder time lookup failed: {e}")))?;
    let mut map: std::collections::HashMap<String, Vec<OffsetDateTime>> =
        std::collections::HashMap::new();
    for (superseded, ts) in rows {
        if let (Some(id), Some(t)) = (superseded, parse_rfc3339(&ts)) {
            map.entry(id).or_default().push(t);
        }
    }
    Ok(map)
}

/// Point-in-time recall (Phase 3.5): the record that was head at `logical_path` as of instant
/// `at`, walking the supersession chain. A record is the as-of head when its effective time is
/// `<= at`, no record whose effective time is `<= at` supersedes it, and (among those) it has the
/// greatest effective time. Returns `None` if nothing existed at the path by then.
pub async fn head_record_as_of(
    store: &BearMemoryStore,
    logical_path: &str,
    at: OffsetDateTime,
) -> Result<Option<MemoryRecordRow>, DenError> {
    let rows = sqlx::query_as::<_, AsOfSqlRow>(
        r"
        SELECT memory_id, sequence_no, scope_type, scope_profile, kind, content_text,
               logical_path, work_surface_ref, metadata_json, created_at,
               supersedes_memory_id, COALESCE(valid_from, created_at) AS effective_at
        FROM memory_records
        WHERE bear_id = ? AND logical_path = ? AND visibility = 'normal'
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(logical_path)
    .fetch_all(store.pool())
    .await
    .map_err(|e| DenError::System(format!("as-of head lookup failed: {e}")))?;

    let parsed: Vec<(AsOfSqlRow, OffsetDateTime)> = rows
        .into_iter()
        .filter_map(|r| parse_rfc3339(&r.effective_at).map(|t| (r, t)))
        .collect();

    // Records superseded by something already effective by `at` are not the as-of head.
    let mut superseded_as_of: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (row, effective) in &parsed {
        if *effective <= at {
            if let Some(prior) = &row.supersedes_memory_id {
                superseded_as_of.insert(prior.clone());
            }
        }
    }

    let best = parsed
        .into_iter()
        .filter(|(row, effective)| {
            *effective <= at && !superseded_as_of.contains(&row.memory_id)
        })
        .max_by(|(a, ta), (b, tb)| ta.cmp(tb).then(a.sequence_no.cmp(&b.sequence_no)));
    Ok(best.map(|(row, _)| row.into_row()))
}

#[derive(sqlx::FromRow)]
struct AsOfSqlRow {
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
    supersedes_memory_id: Option<String>,
    effective_at: String,
}

#[cfg(test)]
mod as_of_tests {
    use super::*;
    use crate::test_support::new_test_store;
    use time::format_description::well_known::Rfc3339;

    fn at(rfc: &str) -> OffsetDateTime {
        OffsetDateTime::parse(rfc, &Rfc3339).unwrap()
    }

    async fn insert_version(
        store: &BearMemoryStore,
        memory_id: &str,
        seq: i64,
        valid_from: &str,
        supersedes: Option<&str>,
    ) {
        sqlx::query(
            r"
            INSERT INTO memory_records (
                memory_id, bear_id, sequence_no, scope_type, scope_profile, kind,
                author_profile, author_agent_id, created_at, content_text, metadata_json,
                visibility, logical_path, work_surface_ref, valid_from, supersedes_memory_id
            ) VALUES (?, ?, ?, 'shared', NULL, 'note', 'curate', NULL, ?, 'policy text', '{}',
                      'normal', 'core/policy.md', NULL, ?, ?)
            ",
        )
        .bind(memory_id)
        .bind(store.bear_id().to_string())
        .bind(seq)
        .bind(valid_from)
        .bind(valid_from)
        .bind(supersedes)
        .execute(store.pool())
        .await
        .expect("insert version");
    }

    #[tokio::test]
    async fn head_record_as_of_walks_supersession_chain() {
        let store = new_test_store().await;
        insert_version(&store, "v1", 1, "2026-01-01T00:00:00Z", None).await;
        insert_version(&store, "v2", 2, "2026-03-01T00:00:00Z", Some("v1")).await;

        // Before anything existed.
        assert!(head_record_as_of(&store, "core/policy.md", at("2025-12-01T00:00:00Z"))
            .await
            .unwrap()
            .is_none());

        // v1 is current until v2 becomes effective.
        let feb = head_record_as_of(&store, "core/policy.md", at("2026-02-01T00:00:00Z"))
            .await
            .unwrap()
            .expect("v1 head in feb");
        assert_eq!(feb.memory_id, "v1");

        // Once v2 is effective it supersedes v1.
        let apr = head_record_as_of(&store, "core/policy.md", at("2026-04-01T00:00:00Z"))
            .await
            .unwrap()
            .expect("v2 head in apr");
        assert_eq!(apr.memory_id, "v2");
    }
}

impl AsOfSqlRow {
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
