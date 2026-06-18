use serde_json::{json, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

use super::records::BearMemoryStore;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SqliteMemoryProposal {
    pub proposal_id: String,
    pub sequence_no: i64,
    pub status: String,
    pub payload_json: Value,
    pub created_at: String,
}

pub async fn create_memory_proposal(
    store: &BearMemoryStore,
    suggested_action: &str,
    sensitivity: &str,
    requires_human: bool,
    payload: &Value,
) -> Result<SqliteMemoryProposal, DenError> {
    let proposal_id = Uuid::new_v4().to_string();
    let sequence_no = store.next_sequence().await?;
    let created_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| DenError::System(format!("timestamp format failed: {e}")))?;
    sqlx::query(
        r"
        INSERT INTO memory_proposals (
            proposal_id, bear_id, sequence_no, suggested_action, sensitivity,
            requires_human, status, payload_json, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)
        ",
    )
    .bind(&proposal_id)
    .bind(store.bear_id().to_string())
    .bind(sequence_no)
    .bind(suggested_action)
    .bind(sensitivity)
    .bind(i32::from(requires_human))
    .bind(payload.to_string())
    .bind(&created_at)
    .execute(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite create proposal failed: {e}")))?;
    Ok(SqliteMemoryProposal {
        proposal_id,
        sequence_no,
        status: "pending".to_string(),
        payload_json: payload.clone(),
        created_at,
    })
}

pub async fn list_memory_proposals(
    store: &BearMemoryStore,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<SqliteMemoryProposal>, DenError> {
    let rows = if let Some(status) = status {
        sqlx::query_as::<_, (String, i64, String, String, String)>(
            r"
            SELECT proposal_id, sequence_no, status, payload_json, created_at
            FROM memory_proposals
            WHERE bear_id = ? AND status = ?
            ORDER BY sequence_no DESC
            LIMIT ?
            ",
        )
        .bind(store.bear_id().to_string())
        .bind(status)
        .bind(limit)
        .fetch_all(store.pool())
        .await
    } else {
        sqlx::query_as::<_, (String, i64, String, String, String)>(
            r"
            SELECT proposal_id, sequence_no, status, payload_json, created_at
            FROM memory_proposals
            WHERE bear_id = ?
            ORDER BY sequence_no DESC
            LIMIT ?
            ",
        )
        .bind(store.bear_id().to_string())
        .bind(limit)
        .fetch_all(store.pool())
        .await
    }
    .map_err(|e| DenError::System(format!("sqlite list proposals failed: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|(proposal_id, sequence_no, status, payload_json, created_at)| {
            SqliteMemoryProposal {
                proposal_id,
                sequence_no,
                status,
                payload_json: serde_json::from_str(&payload_json)
                    .unwrap_or_else(|_| json!({ "raw": payload_json })),
                created_at,
            }
        })
        .collect())
}

pub async fn resolve_memory_proposal(
    store: &BearMemoryStore,
    proposal_id: &str,
    status: &str,
    review_payload: &Value,
) -> Result<SqliteMemoryProposal, DenError> {
    let existing = sqlx::query_as::<_, (String,)>(
        "SELECT payload_json FROM memory_proposals WHERE bear_id = ? AND proposal_id = ?",
    )
    .bind(store.bear_id().to_string())
    .bind(proposal_id)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite fetch proposal for resolve failed: {e}")))?
    .ok_or_else(|| DenError::NotFound("proposal not found".to_string()))?;
    let mut payload: Value = serde_json::from_str(&existing.0).unwrap_or_else(|_| json!({}));
    if let Some(obj) = review_payload.as_object() {
        for (k, v) in obj {
            payload[k] = v.clone();
        }
    }
    let reviewed_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| DenError::System(format!("timestamp format failed: {e}")))?;
    sqlx::query(
        r"
        UPDATE memory_proposals
        SET status = ?, payload_json = ?, reviewed_at = ?
        WHERE bear_id = ? AND proposal_id = ?
        ",
    )
    .bind(status)
    .bind(payload.to_string())
    .bind(&reviewed_at)
    .bind(store.bear_id().to_string())
    .bind(proposal_id)
    .execute(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite resolve proposal failed: {e}")))?;
    let row = sqlx::query_as::<_, (String, i64, String, String, String)>(
        r"
        SELECT proposal_id, sequence_no, status, payload_json, created_at
        FROM memory_proposals WHERE bear_id = ? AND proposal_id = ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(proposal_id)
    .fetch_one(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite fetch proposal failed: {e}")))?;
    Ok(SqliteMemoryProposal {
        proposal_id: row.0,
        sequence_no: row.1,
        status: row.2,
        payload_json: serde_json::from_str(&row.3).unwrap_or_else(|_| json!({})),
        created_at: row.4,
    })
}
