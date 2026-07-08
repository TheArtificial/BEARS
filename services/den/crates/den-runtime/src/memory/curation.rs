//! Routes Bear cognition writes to per-Bear SQLite when `AGENT_RUNTIME=native`.

use den_core::{config::Config, DenError};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use den_memory::{
    self as store, complete_reflection_run_outcome, create_memory_observation,
    create_memory_proposal, create_reflection_run_outcome, list_memory_proposals,
    mark_observation_review_queued, promote_to_shared_core, promote_to_shared_core_at_path,
    resolve_memory_proposal, MemoryStoreManager, SqliteMemoryProposal,
};
use den_service::bears::BearProfile;
use den_service::memory_proposals::{
    CreateMemoryProposal, MemoryProposalRow, ProposalResolutionParams,
};

use crate::bear_observations::{self, BearObservationRow, CreateBearObservation};

pub async fn create_proposal(
    _pool: &PgPool,
    _config: &Config,
    stores: &MemoryStoreManager,
    params: CreateMemoryProposal<'_>,
) -> Result<MemoryProposalRow, DenError> {
    let store = stores.store_for_bear(params.bear_id).await?;
    let payload = json!({
        "source_profile": params.source_profile.as_str(),
        "source_agent_id": params.source_agent_id,
        "source_paths": params.source_paths,
        "source_refs": params.source_refs,
        "target_ref": params.target_ref,
        "title": params.title,
        "summary": params.summary,
        "rationale": params.rationale,
        "proposed_content": params.proposed_content,
        "proposed_patch": params.proposed_patch,
        "refs": params.refs,
        "suggested_action": params.suggested_action,
        "sensitivity": params.sensitivity,
        "requires_human": params.requires_human,
    });
    let sqlite = create_memory_proposal(
        &store,
        params.suggested_action,
        params.sensitivity,
        params.requires_human,
        &payload,
    )
    .await?;
    Ok(sqlite_proposal_to_row(
        params.bear_id,
        &sqlite,
        params.source_profile,
    ))
}

pub async fn create_observation(
    _pool: &PgPool,
    _config: &Config,
    stores: &MemoryStoreManager,
    params: CreateBearObservation<'_>,
) -> Result<BearObservationRow, DenError> {
    let store = stores.store_for_bear(params.bear_id).await?;
    let logical_path = bear_observations::observation_logical_path(params.observation_id);
    let sqlite = create_memory_observation(
        &store,
        params.observation_id,
        params.summary,
        params.salience,
        &logical_path,
        &params.source,
    )
    .await?;
    Ok(sqlite_observation_to_row(params.bear_id, &sqlite))
}

pub async fn get_observation(
    _pool: &PgPool,
    _config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    observation_id: &str,
) -> Result<Option<BearObservationRow>, DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    let sqlite = store::get_memory_observation(&store, observation_id).await?;
    Ok(sqlite.map(|row| sqlite_observation_to_row(bear_id, &row)))
}

pub async fn mark_observation_review_queued_for_bear(
    _config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    observation_id: &str,
    proposal_id: Uuid,
) -> Result<(), DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    mark_observation_review_queued(&store, observation_id, &proposal_id.to_string()).await
}

pub async fn list_proposals(
    _pool: &PgPool,
    _config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<MemoryProposalRow>, DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    let rows = list_memory_proposals(&store, status, limit).await?;
    Ok(rows
        .into_iter()
        .map(|row| sqlite_proposal_to_row(bear_id, &row, BearProfile::Curate))
        .collect())
}

pub async fn get_proposal(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    proposal_id: Uuid,
) -> Result<Option<MemoryProposalRow>, DenError> {
    let proposals = list_proposals(pool, config, stores, bear_id, None, 500).await?;
    Ok(proposals.into_iter().find(|row| row.id == proposal_id))
}

pub async fn resolve_proposal(
    _pool: &PgPool,
    _config: &Config,
    stores: &MemoryStoreManager,
    params: ProposalResolutionParams<'_>,
) -> Result<MemoryProposalRow, DenError> {
    let store = stores.store_for_bear(params.bear_id).await?;
    let review_payload = json!({
        "reviewer_profile": params.reviewer_profile.as_str(),
        "reviewer_agent_id": params.reviewer_agent_id,
        "review_notes": params.review_notes,
        "decision_summary": params.decision_summary,
        "result_path": params.result_path,
        "result_commit": params.result_commit,
    });
    let sqlite = resolve_memory_proposal(
        &store,
        &params.proposal_id.to_string(),
        params.status,
        &review_payload,
    )
    .await?;
    Ok(sqlite_proposal_to_row(
        params.bear_id,
        &sqlite,
        params.reviewer_profile,
    ))
}

pub async fn promote_core_content(
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    source_memory_id: &str,
    kind: &str,
    content_text: &str,
    author_profile: &str,
) -> Result<(String, String), DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    promote_to_shared_core(&store, source_memory_id, kind, content_text, author_profile).await
}

pub async fn promote_core_content_at_path(
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    source_memory_id: &str,
    target_path: &str,
    kind: &str,
    content_text: &str,
    author_profile: &str,
) -> Result<(String, String), DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    promote_to_shared_core_at_path(
        &store,
        source_memory_id,
        target_path,
        kind,
        content_text,
        author_profile,
        None,
    )
    .await
}

pub async fn record_reflection_outcome_start(
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    run_id: &str,
    lane: &str,
    trigger: &str,
    input_summary: Option<&str>,
) -> Result<(), DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    if store::reflection_outcomes::reflection_outcome_exists(&store, run_id).await {
        return Ok(());
    }
    create_reflection_run_outcome(&store, run_id, lane, trigger, input_summary).await
}

pub async fn record_reflection_outcome_complete(
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    run_id: &str,
    status: &str,
    output_summary: Option<&str>,
    proposal_ids: &[String],
) -> Result<(), DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    complete_reflection_run_outcome(&store, run_id, status, output_summary, proposal_ids).await
}

fn sqlite_proposal_to_row(
    bear_id: Uuid,
    sqlite: &SqliteMemoryProposal,
    source_profile: BearProfile,
) -> MemoryProposalRow {
    let p = &sqlite.payload_json;
    MemoryProposalRow {
        id: Uuid::parse_str(&sqlite.proposal_id).unwrap_or_else(|_| Uuid::new_v4()),
        bear_id,
        source_profile: p
            .get("source_profile")
            .and_then(|v| v.as_str())
            .unwrap_or(source_profile.as_str())
            .to_string(),
        source_agent_id: p
            .get("source_agent_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        source_paths: p
            .get("source_paths")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        source_refs: p.get("source_refs").cloned().unwrap_or_else(|| json!({})),
        proposal_type: "memory_review".to_string(),
        suggested_action: p
            .get("suggested_action")
            .and_then(|v| v.as_str())
            .unwrap_or("review")
            .to_string(),
        target_ref: p
            .get("target_ref")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        title: p
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("proposal")
            .to_string(),
        summary: p
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        rationale: p
            .get("rationale")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        proposed_content: p
            .get("proposed_content")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        proposed_patch: p
            .get("proposed_patch")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        refs: p.get("refs").cloned().unwrap_or_else(|| json!({})),
        sensitivity: p
            .get("sensitivity")
            .and_then(|v| v.as_str())
            .unwrap_or("normal")
            .to_string(),
        requires_human: p
            .get("requires_human")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        status: sqlite.status.clone(),
        reviewer_profile: None,
        reviewer_agent_id: None,
        review_notes: None,
        decision_summary: None,
        result_path: None,
        result_commit: None,
        created_at: time::OffsetDateTime::now_utc(),
        reviewed_at: None,
    }
}

fn sqlite_observation_to_row(
    bear_id: Uuid,
    sqlite: &store::SqliteMemoryObservation,
) -> BearObservationRow {
    BearObservationRow {
        id: Uuid::new_v4(),
        bear_id,
        observation_id: sqlite.observation_id.clone(),
        summary: sqlite.summary.clone(),
        salience: "normal".to_string(),
        payload_ref: None,
        source: sqlite.source_json.clone(),
        logical_path: sqlite.logical_path.clone(),
        status: sqlite.status.clone(),
        proposal_id: sqlite
            .proposal_id
            .as_ref()
            .and_then(|id| Uuid::parse_str(id).ok()),
        created_at: time::OffsetDateTime::now_utc(),
        reviewed_at: None,
    }
}
