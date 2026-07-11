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
    let payload = payload_with_dedupe_key(payload, suggested_action, sensitivity);
    if let Some(key) = payload
        .pointer("/dedupe/key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
    {
        if let Some(existing) = pending_proposal_for_dedupe_key(store, key).await? {
            return Ok(existing);
        }
    }

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
        payload_json: payload,
        created_at,
    })
}

async fn pending_proposal_for_dedupe_key(
    store: &BearMemoryStore,
    dedupe_key: &str,
) -> Result<Option<SqliteMemoryProposal>, DenError> {
    let row = sqlx::query_as::<_, (String, i64, String, String, String)>(
        r"
        SELECT proposal_id, sequence_no, status, payload_json, created_at
        FROM memory_proposals
        WHERE bear_id = ?
          AND status = 'pending'
          AND json_extract(payload_json, '$.dedupe.key') = ?
        ORDER BY sequence_no DESC
        LIMIT 1
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(dedupe_key)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite find duplicate proposal failed: {e}")))?;
    Ok(row.map(
        |(proposal_id, sequence_no, status, payload_json, created_at)| SqliteMemoryProposal {
            proposal_id,
            sequence_no,
            status,
            payload_json: serde_json::from_str(&payload_json)
                .unwrap_or_else(|_| json!({ "raw": payload_json })),
            created_at,
        },
    ))
}

fn payload_with_dedupe_key(payload: &Value, suggested_action: &str, sensitivity: &str) -> Value {
    let mut payload = payload.clone();
    let Some(key) = proposal_dedupe_key(&payload, suggested_action, sensitivity) else {
        return payload;
    };
    if !payload.is_object() {
        payload = json!({ "value": payload });
    }
    let obj = payload.as_object_mut().expect("payload object initialized");
    let dedupe = obj.entry("dedupe".to_string()).or_insert_with(|| json!({}));
    if !dedupe.is_object() {
        *dedupe = json!({});
    }
    let dedupe_obj = dedupe.as_object_mut().expect("dedupe object initialized");
    dedupe_obj.insert("key".to_string(), json!(key));
    dedupe_obj.insert("version".to_string(), json!("deterministic-v1"));
    payload
}

fn proposal_dedupe_key(
    payload: &Value,
    suggested_action: &str,
    sensitivity: &str,
) -> Option<String> {
    let target_ref = payload
        .get("target_ref")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let source_hash = payload
        .pointer("/refs/source_hash")
        .or_else(|| payload.pointer("/source_refs/source_hash"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let claim = payload
        .get("proposed_content")
        .and_then(Value::as_str)
        .or_else(|| payload.get("summary").and_then(Value::as_str))
        .map(super::promotions::memory_claim_fingerprint)
        .filter(|value| !value.is_empty());
    let evidence = source_hash.map(str::to_string).or(claim)?;
    // ponytail: deterministic proposal dedupe only; upgrade path is semantic match proposals.
    Some(format!(
        "{}|{}|{}|{}",
        suggested_action.trim().to_ascii_lowercase(),
        sensitivity.trim().to_ascii_lowercase(),
        target_ref,
        evidence
    ))
}

pub async fn get_memory_proposal(
    store: &BearMemoryStore,
    proposal_id: &str,
) -> Result<Option<SqliteMemoryProposal>, DenError> {
    let row = sqlx::query_as::<_, (String, i64, String, String, String)>(
        r"
        SELECT proposal_id, sequence_no, status, payload_json, created_at
        FROM memory_proposals
        WHERE bear_id = ? AND proposal_id = ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(proposal_id)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite get proposal failed: {e}")))?;
    Ok(row.map(
        |(proposal_id, sequence_no, status, payload_json, created_at)| SqliteMemoryProposal {
            proposal_id,
            sequence_no,
            status,
            payload_json: serde_json::from_str(&payload_json)
                .unwrap_or_else(|_| json!({ "raw": payload_json })),
            created_at,
        },
    ))
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
        .map(
            |(proposal_id, sequence_no, status, payload_json, created_at)| SqliteMemoryProposal {
                proposal_id,
                sequence_no,
                status,
                payload_json: serde_json::from_str(&payload_json)
                    .unwrap_or_else(|_| json!({ "raw": payload_json })),
                created_at,
            },
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::new_test_store;

    #[tokio::test]
    async fn pending_duplicate_proposal_reuses_existing_row() {
        let store = new_test_store().await;
        let first = create_memory_proposal(
            &store,
            "human_review",
            "normal",
            true,
            &json!({
                "target_ref": "core/decisions/runtime.md",
                "proposed_content": "Use SQLite as canonical memory.",
                "refs": { "source_hash": "sha256:abc" }
            }),
        )
        .await
        .unwrap();
        let second = create_memory_proposal(
            &store,
            "human_review",
            "normal",
            true,
            &json!({
                "target_ref": "core/decisions/runtime.md",
                "proposed_content": "Different wording should still dedupe by source hash.",
                "refs": { "source_hash": "sha256:abc" }
            }),
        )
        .await
        .unwrap();

        assert_eq!(second.proposal_id, first.proposal_id);
        let proposals: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM memory_proposals WHERE bear_id = ?")
                .bind(store.bear_id().to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(proposals, 1);
    }

    #[tokio::test]
    async fn reviewed_duplicate_claim_can_create_new_proposal() {
        let store = new_test_store().await;
        let first = create_memory_proposal(
            &store,
            "human_review",
            "normal",
            true,
            &json!({
                "target_ref": "core/decisions/runtime.md",
                "proposed_content": "Use SQLite as canonical memory."
            }),
        )
        .await
        .unwrap();
        resolve_memory_proposal(&store, &first.proposal_id, "accepted", &json!({}))
            .await
            .unwrap();

        let second = create_memory_proposal(
            &store,
            "human_review",
            "normal",
            true,
            &json!({
                "target_ref": "core/decisions/runtime.md",
                "proposed_content": "use sqlite as canonical memory"
            }),
        )
        .await
        .unwrap();

        assert_ne!(second.proposal_id, first.proposal_id);
        assert_eq!(
            first
                .payload_json
                .pointer("/dedupe/version")
                .and_then(Value::as_str),
            Some("deterministic-v1")
        );
    }
}
