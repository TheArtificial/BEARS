use sqlx::{PgPool, Row};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

use den_core::DenError;

use crate::runtime::bearwire_projection::wire::BearWireEvent;

#[derive(Debug, Clone)]
pub struct BearWireEventRow {
    pub id: Uuid,
    pub sequence_no: i64,
    pub session_id: String,
    pub event_type: String,
    pub event: BearWireEvent,
    pub created_at: OffsetDateTime,
}

pub async fn append_bearwire_event(
    pool: &PgPool,
    session_id: &str,
    bear_id: Option<Uuid>,
    user_id: Option<i32>,
    mut event: BearWireEvent,
) -> Result<BearWireEventRow, DenError> {
    let initial_json = serde_json::to_value(&event)
        .map_err(|err| DenError::System(format!("serialize BearWire event failed: {err}")))?;
    let row = sqlx::query(
        r#"
        INSERT INTO bearwire_events (session_id, bear_id, user_id, event_type, event_json)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, sequence_no, created_at
        "#,
    )
    .bind(session_id)
    .bind(bear_id)
    .bind(user_id)
    .bind(&event.event_type)
    .bind(initial_json)
    .fetch_one(pool)
    .await?;

    let id: Uuid = row.get("id");
    let sequence_no: i64 = row.get("sequence_no");
    let created_at: OffsetDateTime = row.get("created_at");
    event.event_id = Some(format!("evt_{id}"));
    event.sequence = Some(sequence_no as u64);
    event.time = Some(
        created_at
            .format(&Rfc3339)
            .map_err(|err| DenError::System(format!("format BearWire event time failed: {err}")))?,
    );
    if event.session_id.is_none() {
        event.session_id = Some(session_id.to_string());
    }

    let final_json = serde_json::to_value(&event)
        .map_err(|err| DenError::System(format!("serialize BearWire event failed: {err}")))?;
    sqlx::query("UPDATE bearwire_events SET event_json = $2 WHERE id = $1")
        .bind(id)
        .bind(final_json)
        .execute(pool)
        .await?;

    Ok(BearWireEventRow {
        id,
        sequence_no,
        session_id: session_id.to_string(),
        event_type: event.event_type.clone(),
        event,
        created_at,
    })
}

pub async fn list_bearwire_events_after(
    pool: &PgPool,
    session_id: &str,
    after_sequence: Option<i64>,
    limit: i64,
) -> Result<Vec<BearWireEventRow>, DenError> {
    let limit = limit.clamp(1, 500);
    let rows = sqlx::query(
        r#"
        SELECT id, sequence_no, session_id, event_type, event_json, created_at
        FROM bearwire_events
        WHERE session_id = $1
          AND ($2::bigint IS NULL OR sequence_no > $2)
        ORDER BY sequence_no ASC
        LIMIT $3
        "#,
    )
    .bind(session_id)
    .bind(after_sequence)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let event_json: serde_json::Value = row.get("event_json");
            let event: BearWireEvent = serde_json::from_value(event_json)
                .map_err(|err| DenError::System(format!("decode BearWire event failed: {err}")))?;
            Ok(BearWireEventRow {
                id: row.get("id"),
                sequence_no: row.get("sequence_no"),
                session_id: row.get("session_id"),
                event_type: row.get("event_type"),
                event,
                created_at: row.get("created_at"),
            })
        })
        .collect()
}
