use time::OffsetDateTime;
use uuid::Uuid;

use crate::errors::CustomError;

use super::records::BearMemoryStore;

#[derive(Debug, Clone)]
pub struct MemoryLinkRow {
    pub link_id: String,
    pub sequence_no: i64,
    pub src_memory_id: String,
    pub dst_ref_type: String,
    pub dst_ref: String,
    pub link_type: String,
    pub created_at: String,
}

pub async fn append_memory_link(
    store: &BearMemoryStore,
    src_memory_id: &str,
    dst_ref_type: &str,
    dst_ref: &str,
    link_type: &str,
) -> Result<String, CustomError> {
    let link_id = Uuid::new_v4().to_string();
    let sequence_no = store.next_sequence().await?;
    let created_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| CustomError::System(format!("timestamp format failed: {e}")))?;
    sqlx::query(
        r#"
        INSERT INTO memory_links (
            link_id, bear_id, sequence_no, src_memory_id, dst_ref_type, dst_ref,
            link_type, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&link_id)
    .bind(store.bear_id().to_string())
    .bind(sequence_no)
    .bind(src_memory_id)
    .bind(dst_ref_type)
    .bind(dst_ref)
    .bind(link_type)
    .bind(&created_at)
    .execute(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("sqlite append memory_link failed: {e}")))?;
    Ok(link_id)
}

pub async fn list_memory_links_for_source(
    store: &BearMemoryStore,
    src_memory_id: &str,
    limit: i64,
) -> Result<Vec<MemoryLinkRow>, CustomError> {
    let rows = sqlx::query_as::<_, MemoryLinkSqlRow>(
        r#"
        SELECT link_id, sequence_no, src_memory_id, dst_ref_type, dst_ref, link_type, created_at
        FROM memory_links
        WHERE bear_id = ? AND src_memory_id = ?
        ORDER BY sequence_no DESC
        LIMIT ?
        "#,
    )
    .bind(store.bear_id().to_string())
    .bind(src_memory_id)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("sqlite list memory_links failed: {e}")))?;
    Ok(rows.into_iter().map(MemoryLinkSqlRow::into_row).collect())
}

pub async fn list_memory_links_for_bear(
    store: &BearMemoryStore,
    limit: i64,
) -> Result<Vec<MemoryLinkRow>, CustomError> {
    let rows = sqlx::query_as::<_, MemoryLinkSqlRow>(
        r#"
        SELECT link_id, sequence_no, src_memory_id, dst_ref_type, dst_ref, link_type, created_at
        FROM memory_links
        WHERE bear_id = ?
        ORDER BY sequence_no DESC
        LIMIT ?
        "#,
    )
    .bind(store.bear_id().to_string())
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("sqlite list memory_links failed: {e}")))?;
    Ok(rows.into_iter().map(MemoryLinkSqlRow::into_row).collect())
}

#[derive(sqlx::FromRow)]
struct MemoryLinkSqlRow {
    link_id: String,
    sequence_no: i64,
    src_memory_id: String,
    dst_ref_type: String,
    dst_ref: String,
    link_type: String,
    created_at: String,
}

impl MemoryLinkSqlRow {
    fn into_row(self) -> MemoryLinkRow {
        MemoryLinkRow {
            link_id: self.link_id,
            sequence_no: self.sequence_no,
            src_memory_id: self.src_memory_id,
            dst_ref_type: self.dst_ref_type,
            dst_ref: self.dst_ref,
            link_type: self.link_type,
            created_at: self.created_at,
        }
    }
}
