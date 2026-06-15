use serde_json::Value;
use time::OffsetDateTime;

use den_core::DenError;

use super::records::BearMemoryStore;

#[derive(Debug, Clone)]
pub struct SqliteMemoryObservation {
    pub observation_id: String,
    pub sequence_no: i64,
    pub summary: String,
    pub logical_path: String,
    pub status: String,
    pub proposal_id: Option<String>,
    pub source_json: Value,
    pub created_at: String,
}

pub async fn create_memory_observation(
    store: &BearMemoryStore,
    observation_id: &str,
    summary: &str,
    salience: &str,
    logical_path: &str,
    source: &Value,
) -> Result<SqliteMemoryObservation, DenError> {
    let sequence_no = store.next_sequence().await?;
    let created_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| DenError::System(format!("timestamp format failed: {e}")))?;
    sqlx::query(
        r"
        INSERT INTO memory_observations (
            observation_id, bear_id, sequence_no, summary, salience, logical_path,
            source_json, status, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, 'pending_review', ?)
        ",
    )
    .bind(observation_id)
    .bind(store.bear_id().to_string())
    .bind(sequence_no)
    .bind(summary)
    .bind(salience)
    .bind(logical_path)
    .bind(source.to_string())
    .bind(&created_at)
    .execute(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite create observation failed: {e}")))?;
    Ok(SqliteMemoryObservation {
        observation_id: observation_id.to_string(),
        sequence_no,
        summary: summary.to_string(),
        logical_path: logical_path.to_string(),
        status: "pending_review".to_string(),
        proposal_id: None,
        source_json: source.clone(),
        created_at,
    })
}

pub async fn get_memory_observation(
    store: &BearMemoryStore,
    observation_id: &str,
) -> Result<Option<SqliteMemoryObservation>, DenError> {
    let row = sqlx::query_as::<_, ObservationSqlRow>(
        r"
        SELECT observation_id, sequence_no, summary, logical_path, status,
               proposal_id, source_json, created_at
        FROM memory_observations
        WHERE bear_id = ? AND observation_id = ?
        ",
    )
    .bind(store.bear_id().to_string())
    .bind(observation_id)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite get observation failed: {e}")))?;
    Ok(row.map(ObservationSqlRow::into_row))
}

pub async fn mark_observation_review_queued(
    store: &BearMemoryStore,
    observation_id: &str,
    proposal_id: &str,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE memory_observations
        SET status = 'review_queued', proposal_id = ?
        WHERE bear_id = ? AND observation_id = ?
        ",
    )
    .bind(proposal_id)
    .bind(store.bear_id().to_string())
    .bind(observation_id)
    .execute(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite mark observation queued failed: {e}")))?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ObservationSqlRow {
    observation_id: String,
    sequence_no: i64,
    summary: String,
    logical_path: String,
    status: String,
    proposal_id: Option<String>,
    source_json: String,
    created_at: String,
}

impl ObservationSqlRow {
    fn into_row(self) -> SqliteMemoryObservation {
        SqliteMemoryObservation {
            observation_id: self.observation_id,
            sequence_no: self.sequence_no,
            summary: self.summary,
            logical_path: self.logical_path,
            status: self.status,
            proposal_id: self.proposal_id,
            source_json: serde_json::from_str(&self.source_json)
                .unwrap_or_else(|_| serde_json::json!({})),
            created_at: self.created_at,
        }
    }
}
