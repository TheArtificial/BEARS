use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BearObservationRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub observation_id: String,
    pub summary: String,
    pub salience: String,
    pub payload_ref: Option<String>,
    pub source: serde_json::Value,
    pub logical_path: String,
    pub status: String,
    pub proposal_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub reviewed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone)]
pub struct CreateBearObservation<'a> {
    pub bear_id: Uuid,
    pub observation_id: &'a str,
    pub summary: &'a str,
    pub salience: &'a str,
    pub payload_ref: Option<&'a str>,
    pub source: serde_json::Value,
}

pub fn observation_logical_path(observation_id: &str) -> String {
    format!("watch/observations/{observation_id}.md")
}

pub async fn create(
    pool: &PgPool,
    params: CreateBearObservation<'_>,
) -> Result<BearObservationRow, DenError> {
    let logical_path = observation_logical_path(params.observation_id);
    let row = sqlx::query(
        r#"
        INSERT INTO bear_observations (
            bear_id, observation_id, summary, salience, payload_ref, source, logical_path
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id, bear_id, observation_id, summary, salience, payload_ref, source,
                  logical_path, status, proposal_id, created_at, reviewed_at
        "#,
    )
    .bind(params.bear_id)
    .bind(params.observation_id)
    .bind(params.summary)
    .bind(params.salience)
    .bind(params.payload_ref)
    .bind(params.source)
    .bind(logical_path)
    .fetch_one(pool)
    .await?;
    Ok(row_from_sql(row))
}

pub async fn mark_review_queued(
    pool: &PgPool,
    bear_id: Uuid,
    observation_row_id: Uuid,
    proposal_id: Uuid,
) -> Result<BearObservationRow, DenError> {
    let row = sqlx::query(
        r#"
        UPDATE bear_observations
        SET status = 'review_queued',
            proposal_id = $3
        WHERE bear_id = $1 AND id = $2
        RETURNING id, bear_id, observation_id, summary, salience, payload_ref, source,
                  logical_path, status, proposal_id, created_at, reviewed_at
        "#,
    )
    .bind(bear_id)
    .bind(observation_row_id)
    .bind(proposal_id)
    .fetch_one(pool)
    .await?;
    Ok(row_from_sql(row))
}

pub async fn get_for_bear(
    pool: &PgPool,
    bear_id: Uuid,
    observation_id: &str,
) -> Result<Option<BearObservationRow>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, bear_id, observation_id, summary, salience, payload_ref, source,
               logical_path, status, proposal_id, created_at, reviewed_at
        FROM bear_observations
        WHERE bear_id = $1 AND observation_id = $2
        "#,
    )
    .bind(bear_id)
    .bind(observation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_from_sql))
}

fn row_from_sql(row: sqlx::postgres::PgRow) -> BearObservationRow {
    BearObservationRow {
        id: row.get("id"),
        bear_id: row.get("bear_id"),
        observation_id: row.get("observation_id"),
        summary: row.get("summary"),
        salience: row.get("salience"),
        payload_ref: row.get("payload_ref"),
        source: row.get("source"),
        logical_path: row.get("logical_path"),
        status: row.get("status"),
        proposal_id: row.get("proposal_id"),
        created_at: row.get("created_at"),
        reviewed_at: row.get("reviewed_at"),
    }
}
