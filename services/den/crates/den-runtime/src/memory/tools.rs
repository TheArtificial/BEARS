use den_core::DenError;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    memory::store::{
        append_memory_record, list_records_for_logical_path, LogicalMemoryPath, MemoryStoreManager,
    },
};

use super::store::BearMemoryStore;

pub async fn sqlite_write_at_path(
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    logical_path: &str,
    author_profile: &str,
    title: &str,
    body: &str,
    metadata: Value,
) -> Result<Value, DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    let logical = LogicalMemoryPath::from_logical_path(logical_path);
    let content = if body.starts_with('#') {
        body.to_string()
    } else {
        format!("# {title}\n\n{body}")
    };
    let mut metadata_obj = metadata.as_object().cloned().unwrap_or_default();
    metadata_obj.insert("title".to_string(), json!(title));
    metadata_obj.insert("storage".to_string(), json!("sqlite"));
    metadata_obj.insert("runtime".to_string(), json!("native"));
    let row = append_memory_record(
        &store,
        &logical,
        &logical.kind,
        author_profile,
        None,
        &content,
        &Value::Object(metadata_obj),
    )
    .await?;
    Ok(json!({
        "bear_id": bear_id,
        "profile": author_profile,
        "kind": row.kind,
        "entry_id": row.memory_id,
        "path": row.logical_path,
        "sequence_no": row.sequence_no,
        "storage": "sqlite",
    }))
}

pub async fn sqlite_write_profile_entry(
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    profile: &str,
    kind: &str,
    title: &str,
    body: &str,
    tags: &[String],
    source: Option<Value>,
    author: Option<String>,
) -> Result<Value, DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    let logical = LogicalMemoryPath::profile_local(profile, kind);
    let content = format!("# {title}\n\n{body}");
    let metadata = json!({
        "title": title,
        "tags": tags,
        "source": source,
        "author": author,
        "storage": "sqlite",
        "runtime": "native",
    });
    let row = append_memory_record(
        &store,
        &logical,
        kind,
        profile,
        None,
        &content,
        &metadata,
    )
    .await?;
    Ok(json!({
        "bear_id": bear_id,
        "profile": profile,
        "kind": row.kind,
        "entry_id": row.memory_id,
        "path": row.logical_path,
        "sequence_no": row.sequence_no,
        "storage": "sqlite",
    }))
}

pub async fn sqlite_memory_browse(
    store: &BearMemoryStore,
    role: &str,
) -> Result<Value, DenError> {
    let rows = sqlx::query_scalar::<_, String>(
        r"
        SELECT DISTINCT logical_path
        FROM memory_records
        WHERE bear_id = ? AND scope_profile = ? AND logical_path IS NOT NULL
        ORDER BY logical_path ASC
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(role)
    .fetch_all(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite memory browse failed: {e}")))?;
    let children: Vec<Value> = rows
        .into_iter()
        .map(|path| {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            json!({
                "name": name,
                "path": path,
                "type": "file",
            })
        })
        .collect();
    Ok(json!({
        "ok": true,
        "configured": true,
        "storage": "sqlite",
        "role": role,
        "children": children,
    }))
}

pub async fn sqlite_memory_read(
    store: &BearMemoryStore,
    logical_path: &str,
) -> Result<Value, DenError> {
    let rows = list_records_for_logical_path(store, logical_path, 20).await?;
    if rows.is_empty() {
        return Ok(json!({
            "ok": false,
            "configured": true,
            "storage": "sqlite",
            "path": logical_path,
            "message": "no records at path",
        }));
    }
    let body: String = rows
        .iter()
        .rev()
        .map(|r| r.content_text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    Ok(json!({
        "ok": true,
        "configured": true,
        "storage": "sqlite",
        "path": logical_path,
        "content": body,
        "record_count": rows.len(),
        "latest_sequence_no": rows.first().map(|r| r.sequence_no),
    }))
}

/// Keyword (`LIKE`) memory search — the fallback path for `memory_search` when the derived
/// recall index is unavailable. Scoped to memory **visible to `role`**: shared (core) records
/// OR this role's own profile-local records (matching the vector path's `role_scope_filter`, so
/// both strategies honor the same role-local boundary). Provenance mirrors the vector path
/// (`memory_id`, `path`, `snippet`); `score` is `null` since keyword matching is unranked.
pub async fn sqlite_memory_search(
    store: &BearMemoryStore,
    role: &str,
    query: &str,
    limit: i64,
) -> Result<Value, DenError> {
    let pattern = format!("%{query}%");
    let rows = sqlx::query_as::<_, (String, Option<String>, String, i64, String)>(
        r"
        SELECT memory_id, logical_path, content_text, sequence_no, kind
        FROM memory_records
        WHERE bear_id = ?
          AND (scope_type = 'shared' OR scope_profile = ?)
          AND content_text LIKE ?
        ORDER BY sequence_no DESC
        LIMIT ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(role)
    .bind(pattern)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite memory search failed: {e}")))?;
    let hits: Vec<Value> = rows
        .into_iter()
        .map(|(memory_id, path, content, sequence_no, kind)| {
            json!({
                "memory_id": memory_id,
                "path": path,
                "kind": kind,
                "score": Value::Null,
                "snippet": content.chars().take(240).collect::<String>(),
                "sequence_no": sequence_no,
            })
        })
        .collect();
    Ok(json!({
        "ok": true,
        "configured": true,
        "storage": "sqlite",
        "strategy": "keyword",
        "query": query,
        "hits": hits,
    }))
}

pub async fn sqlite_collect_role_logical_paths(
    store: &BearMemoryStore,
    role: &str,
) -> Result<Vec<String>, DenError> {
    sqlx::query_scalar::<_, String>(
        r"
        SELECT DISTINCT logical_path
        FROM memory_records
        WHERE bear_id = ? AND scope_profile = ? AND logical_path IS NOT NULL
        ORDER BY logical_path ASC
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(role)
    .fetch_all(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite collect paths failed: {e}")))
}

pub async fn sqlite_list_plan_artifacts(
    store: &BearMemoryStore,
    role: &str,
    limit: i64,
) -> Result<Value, DenError> {
    let rows = sqlx::query_as::<_, (String, String, String, i64)>(
        r"
        SELECT memory_id, logical_path, content_text, sequence_no
        FROM memory_records
        WHERE bear_id = ? AND scope_profile = ? AND logical_path LIKE ?
        ORDER BY sequence_no DESC
        LIMIT ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(role)
    .bind(format!("{role}/plans/%"))
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite list plan artifacts failed: {e}")))?;
    let results: Vec<Value> = rows
        .into_iter()
        .map(|(memory_id, path, content, sequence_no)| {
            json!({
                "memory_id": memory_id,
                "path": path,
                "snippet": content.chars().take(240).collect::<String>(),
                "sequence_no": sequence_no,
                "storage": "sqlite",
            })
        })
        .collect();
    Ok(json!(results))
}

pub async fn sqlite_memory_status(
    store: &BearMemoryStore,
    role: &str,
) -> Result<Value, DenError> {
    let file_count: i64 = sqlx::query_scalar(
        r"
        SELECT COUNT(DISTINCT logical_path)
        FROM memory_records
        WHERE bear_id = ? AND scope_profile = ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(role)
    .fetch_one(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite memory status failed: {e}")))?;
    Ok(json!({
        "configured": true,
        "available": true,
        "storage": "sqlite",
        "role": role,
        "file_count": file_count,
    }))
}
