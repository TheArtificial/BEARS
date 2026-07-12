use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use den_core::DenError;

use super::{clock::now_rfc3339, records::BearMemoryStore};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MemoryHarvestMark {
    pub mark_id: String,
    pub bear_id: String,
    pub sequence_no: i64,
    pub source_kind: String,
    pub source_ref: String,
    pub source_hash: Option<String>,
    pub harvested_at: String,
    pub run_id: Option<String>,
    pub proposal_ids_json: String,
}

pub async fn record_harvest_mark(
    store: &BearMemoryStore,
    source_kind: &str,
    source_ref: &str,
    source_hash: Option<&str>,
    run_id: Option<&str>,
    proposal_ids: &[String],
) -> Result<MemoryHarvestMark, DenError> {
    let mark_id = Uuid::new_v4().to_string();
    let sequence_no = store.next_sequence().await?;
    let harvested_at = now_rfc3339()?;
    let proposal_ids_json = serde_json::to_string(proposal_ids)
        .map_err(|e| DenError::System(format!("encode proposal ids failed: {e}")))?;

    sqlx::query(
        r"
        INSERT INTO memory_harvest_marks (
            mark_id, bear_id, sequence_no, source_kind, source_ref, source_hash,
            harvested_at, run_id, proposal_ids_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(bear_id, source_kind, source_ref) DO UPDATE SET
            source_hash = excluded.source_hash,
            harvested_at = excluded.harvested_at,
            run_id = excluded.run_id,
            proposal_ids_json = excluded.proposal_ids_json
        ",
    )
    .bind(&mark_id)
    .bind(store.bear_id().to_string())
    .bind(sequence_no)
    .bind(source_kind)
    .bind(source_ref)
    .bind(source_hash)
    .bind(&harvested_at)
    .bind(run_id)
    .bind(&proposal_ids_json)
    .execute(store.pool())
    .await
    .map_err(|e| DenError::System(format!("record harvest mark failed: {e}")))?;

    harvest_mark_for_source(store, source_kind, source_ref)
        .await?
        .ok_or_else(|| DenError::System("harvest mark missing after write".to_string()))
}

pub async fn harvest_mark_for_source(
    store: &BearMemoryStore,
    source_kind: &str,
    source_ref: &str,
) -> Result<Option<MemoryHarvestMark>, DenError> {
    sqlx::query_as::<_, MemoryHarvestMark>(
        r"
        SELECT mark_id, bear_id, sequence_no, source_kind, source_ref, source_hash,
               harvested_at, run_id, proposal_ids_json
        FROM memory_harvest_marks
        WHERE bear_id = ? AND source_kind = ? AND source_ref = ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(source_kind)
    .bind(source_ref)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| DenError::System(format!("get harvest mark failed: {e}")))
}

pub async fn harvest_source_marked(
    store: &BearMemoryStore,
    source_kind: &str,
    source_ref: &str,
) -> Result<bool, DenError> {
    let n: i64 = sqlx::query_scalar(
        r"
        SELECT COUNT(*)
        FROM memory_harvest_marks
        WHERE bear_id = ? AND source_kind = ? AND source_ref = ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(source_kind)
    .bind(source_ref)
    .fetch_one(store.pool())
    .await
    .map_err(|e| DenError::System(format!("check harvest mark failed: {e}")))?;
    Ok(n > 0)
}

pub async fn harvest_source_hash_marked(
    store: &BearMemoryStore,
    source_kind: &str,
    source_hash: &str,
) -> Result<bool, DenError> {
    let n: i64 = sqlx::query_scalar(
        r"
        SELECT COUNT(*)
        FROM memory_harvest_marks
        WHERE bear_id = ? AND source_kind = ? AND source_hash = ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(source_kind)
    .bind(source_hash)
    .fetch_one(store.pool())
    .await
    .map_err(|e| DenError::System(format!("check harvest mark hash failed: {e}")))?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::new_test_store;

    #[tokio::test]
    async fn harvest_mark_is_idempotent_by_source() {
        let store = new_test_store().await;
        let first = record_harvest_mark(
            &store,
            "conversation",
            "den-conv-1",
            Some("h1"),
            Some("run-1"),
            &["proposal-1".to_string()],
        )
        .await
        .unwrap();
        let second = record_harvest_mark(
            &store,
            "conversation",
            "den-conv-1",
            Some("h2"),
            Some("run-2"),
            &["proposal-2".to_string()],
        )
        .await
        .unwrap();

        assert_eq!(first.source_ref, second.source_ref);
        assert_eq!(second.source_hash.as_deref(), Some("h2"));
        assert!(harvest_source_marked(&store, "conversation", "den-conv-1")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn harvest_mark_can_be_checked_by_source_hash() {
        let store = new_test_store().await;
        record_harvest_mark(
            &store,
            "compaction_artifact",
            "artifact-1",
            Some("sha256:same-content"),
            Some("run-1"),
            &[],
        )
        .await
        .unwrap();

        assert!(
            harvest_source_hash_marked(&store, "compaction_artifact", "sha256:same-content")
                .await
                .unwrap()
        );
        assert!(!harvest_source_hash_marked(
            &store,
            "compaction_artifact",
            "sha256:different-content"
        )
        .await
        .unwrap());
    }
}
