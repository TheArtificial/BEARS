use time::OffsetDateTime;

use den_core::DenError;

use super::records::BearMemoryStore;

pub async fn create_reflection_run_outcome(
    store: &BearMemoryStore,
    run_id: &str,
    lane: &str,
    trigger: &str,
    input_summary: Option<&str>,
) -> Result<(), DenError> {
    let sequence_no = store.next_sequence().await?;
    let created_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| DenError::System(format!("timestamp format failed: {e}")))?;
    sqlx::query(
        r"
        INSERT INTO reflection_run_outcomes (
            run_id, bear_id, sequence_no, lane, trigger, status,
            input_summary, proposal_ids_json, created_at
        ) VALUES (?, ?, ?, ?, ?, 'running', ?, '[]', ?)
        ",
    )
    .bind(run_id)
    .bind(store.bear_id().to_string())
    .bind(sequence_no)
    .bind(lane)
    .bind(trigger)
    .bind(input_summary)
    .bind(&created_at)
    .execute(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite create reflection outcome failed: {e}")))?;
    Ok(())
}

pub async fn complete_reflection_run_outcome(
    store: &BearMemoryStore,
    run_id: &str,
    status: &str,
    output_summary: Option<&str>,
    proposal_ids: &[String],
) -> Result<(), DenError> {
    let completed_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| DenError::System(format!("timestamp format failed: {e}")))?;
    let ids_json = serde_json::to_string(proposal_ids)
        .map_err(|e| DenError::System(format!("proposal ids json failed: {e}")))?;
    sqlx::query(
        r"
        UPDATE reflection_run_outcomes
        SET status = ?, output_summary = ?, proposal_ids_json = ?, completed_at = ?
        WHERE bear_id = ? AND run_id = ?
        ",
    )
    .bind(status)
    .bind(output_summary)
    .bind(ids_json)
    .bind(&completed_at)
    .bind(store.bear_id().to_string())
    .bind(run_id)
    .execute(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite complete reflection outcome failed: {e}")))?;
    Ok(())
}

pub async fn reflection_outcome_exists(
    store: &BearMemoryStore,
    run_id: &str,
) -> Result<bool, DenError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reflection_run_outcomes WHERE bear_id = ? AND run_id = ?",
    )
    .bind(store.bear_id().to_string())
    .bind(run_id)
    .fetch_one(store.pool())
    .await
    .map_err(|e| DenError::System(format!("sqlite check reflection outcome failed: {e}")))?;
    Ok(count > 0)
}
