use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    core::{
        bears::BearAgentRole,
        conversation_events::{
            canonical_persistence_context, spawn_persist_assistant_summary_message,
            spawn_persist_workflow_event, ConversationEventProvenance,
        },
    },
    errors::CustomError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProposalRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub source_role: String,
    pub source_agent_id: Option<String>,
    pub source_paths: Vec<String>,
    pub source_refs: serde_json::Value,
    pub proposal_type: String,
    pub suggested_action: String,
    pub target_ref: Option<String>,
    pub title: String,
    pub summary: String,
    pub rationale: String,
    pub proposed_content: Option<String>,
    pub proposed_patch: Option<String>,
    pub refs: serde_json::Value,
    pub sensitivity: String,
    pub requires_human: bool,
    pub status: String,
    pub reviewer_role: Option<String>,
    pub reviewer_agent_id: Option<String>,
    pub review_notes: Option<String>,
    pub decision_summary: Option<String>,
    pub result_path: Option<String>,
    pub result_commit: Option<String>,
    pub created_at: OffsetDateTime,
    pub reviewed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone)]
pub struct CreateMemoryProposal<'a> {
    pub bear_id: Uuid,
    pub source_role: BearAgentRole,
    pub source_agent_id: Option<String>,
    pub source_paths: Vec<String>,
    pub source_refs: serde_json::Value,
    pub suggested_action: &'a str,
    pub target_ref: Option<&'a str>,
    pub title: &'a str,
    pub summary: &'a str,
    pub rationale: &'a str,
    pub proposed_content: Option<&'a str>,
    pub proposed_patch: Option<&'a str>,
    pub refs: serde_json::Value,
    pub sensitivity: &'a str,
    pub requires_human: bool,
}

fn memory_proposal_provenance(bear_id: Uuid) -> ConversationEventProvenance {
    ConversationEventProvenance {
        source: "memory_proposals".to_string(),
        scope_id: format!("bear:{bear_id}"),
    }
}

fn conversation_id_for_proposal(row: &MemoryProposalRow) -> Option<&str> {
    row.source_refs
        .get("conversation_id")
        .and_then(|value| value.as_str())
        .filter(|value| value.starts_with("conv-"))
}

fn review_projection_context(
    pool: &PgPool,
    row: &MemoryProposalRow,
) -> Option<(crate::core::conversation_events::ConversationPersistenceContext, ConversationEventProvenance)> {
    let conversation_id = conversation_id_for_proposal(row)?;
    let provenance = memory_proposal_provenance(row.bear_id);
    let context = canonical_persistence_context(
        pool.clone(),
        row.bear_id,
        None,
        conversation_id.to_string(),
        None,
        None,
        provenance.scope_id.clone(),
        false,
    );
    Some((context, provenance))
}

fn maybe_project_memory_proposal_lifecycle(
    pool: &PgPool,
    row: &MemoryProposalRow,
    event: &str,
    workflow_text: String,
    summary_text: Option<String>,
    extra_json: serde_json::Value,
) {
    let Some((context, provenance)) = review_projection_context(pool, row) else {
        return;
    };
    let mut content_json = serde_json::json!({
        "source": provenance.source,
        "event": event,
        "scope_id": provenance.scope_id,
        "proposal_id": row.id,
        "source_role": row.source_role,
        "suggested_action": row.suggested_action,
        "title": row.title,
        "status": row.status,
    });
    if let (Some(base), Some(extra)) = (content_json.as_object_mut(), extra_json.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    spawn_persist_workflow_event(context.clone(), workflow_text, content_json, None);
    if let Some(summary_text) = summary_text {
        spawn_persist_assistant_summary_message(context, summary_text, None);
    }
}

fn maybe_spawn_memory_proposal_created_event(pool: &PgPool, row: &MemoryProposalRow) {
    maybe_project_memory_proposal_lifecycle(
        pool,
        row,
        "memory_proposal_created",
        format!("Memory proposal created: {}", row.title),
        Some(format!(
            "Review requested for memory proposal '{}' from {}.",
            row.title, row.source_role
        )),
        serde_json::json!({}),
    );
}

pub async fn create(
    pool: &PgPool,
    params: CreateMemoryProposal<'_>,
) -> Result<MemoryProposalRow, CustomError> {
    let row = sqlx::query(
        r#"
        INSERT INTO bear_memory_proposals (
            bear_id, source_role, source_agent_id, source_paths, source_refs,
            proposal_type, suggested_action, target_ref, title, summary, rationale,
            proposed_content, proposed_patch, refs, sensitivity, requires_human, status
        )
        VALUES (
            $1, $2, $3, $4, $5, 'memory_review', $6, $7, $8, $9, $10,
            $11, $12, $13, $14, $15, 'pending'
        )
        RETURNING id, bear_id, source_role, source_agent_id, source_paths, source_refs,
                  proposal_type, suggested_action, target_ref, title, summary, rationale,
                  proposed_content, proposed_patch, refs, sensitivity, requires_human, status,
                  reviewer_role, reviewer_agent_id, review_notes, decision_summary,
                  result_path, result_commit, created_at, reviewed_at
        "#,
    )
    .bind(params.bear_id)
    .bind(params.source_role.as_str())
    .bind(params.source_agent_id)
    .bind(params.source_paths)
    .bind(params.source_refs)
    .bind(params.suggested_action)
    .bind(params.target_ref)
    .bind(params.title)
    .bind(params.summary)
    .bind(params.rationale)
    .bind(params.proposed_content)
    .bind(params.proposed_patch)
    .bind(params.refs)
    .bind(params.sensitivity)
    .bind(params.requires_human)
    .fetch_one(pool)
    .await?;
    let proposal = row_from_sql(row);
    maybe_spawn_memory_proposal_created_event(pool, &proposal);
    Ok(proposal)
}

pub async fn list_for_bear(
    pool: &PgPool,
    bear_id: Uuid,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<MemoryProposalRow>, CustomError> {
    let rows = sqlx::query(
        r#"
        SELECT id, bear_id, source_role, source_agent_id, source_paths, source_refs,
               proposal_type, suggested_action, target_ref, title, summary, rationale,
               proposed_content, proposed_patch, refs, sensitivity, requires_human, status,
               reviewer_role, reviewer_agent_id, review_notes, decision_summary,
               result_path, result_commit, created_at, reviewed_at
        FROM bear_memory_proposals
        WHERE bear_id = $1
          AND ($2::text IS NULL OR status = $2)
        ORDER BY created_at DESC
        LIMIT $3
        "#,
    )
    .bind(bear_id)
    .bind(status)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_from_sql).collect())
}

pub struct ProposalResolutionParams<'a> {
    pub bear_id: Uuid,
    pub proposal_id: Uuid,
    pub reviewer_role: BearAgentRole,
    pub reviewer_agent_id: Option<&'a str>,
    pub status: &'a str,
    pub review_notes: Option<&'a str>,
    pub decision_summary: Option<&'a str>,
    pub result_path: Option<&'a str>,
    pub result_commit: Option<&'a str>,
}

pub async fn resolve_for_bear(
    pool: &PgPool,
    params: ProposalResolutionParams<'_>,
) -> Result<MemoryProposalRow, CustomError> {
    let row = sqlx::query(
        r#"
        UPDATE bear_memory_proposals
        SET status = $3,
            reviewer_role = $4,
            reviewer_agent_id = $5,
            review_notes = $6,
            decision_summary = $7,
            result_path = COALESCE($8, result_path),
            result_commit = COALESCE($9, result_commit),
            reviewed_at = NOW()
        WHERE bear_id = $1 AND id = $2
        RETURNING id, bear_id, source_role, source_agent_id, source_paths, source_refs,
                  proposal_type, suggested_action, target_ref, title, summary, rationale,
                  proposed_content, proposed_patch, refs, sensitivity, requires_human, status,
                  reviewer_role, reviewer_agent_id, review_notes, decision_summary,
                  result_path, result_commit, created_at, reviewed_at
        "#,
    )
    .bind(params.bear_id)
    .bind(params.proposal_id)
    .bind(params.status)
    .bind(params.reviewer_role.as_str())
    .bind(params.reviewer_agent_id)
    .bind(params.review_notes)
    .bind(params.decision_summary)
    .bind(params.result_path)
    .bind(params.result_commit)
    .fetch_one(pool)
    .await?;
    let proposal = row_from_sql(row);
    let summary_text = match proposal.status.as_str() {
        "approved" => Some(format!(
            "Memory proposal '{}' was approved{}.",
            proposal.title,
            proposal
                .result_path
                .as_deref()
                .map(|path| format!(" and applied at {path}"))
                .unwrap_or_default()
        )),
        "rejected" => Some(format!("Memory proposal '{}' was rejected.", proposal.title)),
        "deferred" => Some(format!("Memory proposal '{}' was deferred.", proposal.title)),
        "retained_local" => Some(format!(
            "Memory proposal '{}' was retained as role-local only.",
            proposal.title
        )),
        "superseded" => Some(format!("Memory proposal '{}' was superseded.", proposal.title)),
        "needs_human_review" => Some(format!(
            "Memory proposal '{}' now requires human review.",
            proposal.title
        )),
        _ => None,
    };
    maybe_project_memory_proposal_lifecycle(
        pool,
        &proposal,
        "memory_proposal_resolved",
        format!("Memory proposal resolved: {}", proposal.title),
        summary_text,
        serde_json::json!({
            "reviewer_role": proposal.reviewer_role,
            "reviewer_agent_id": proposal.reviewer_agent_id,
            "review_notes": proposal.review_notes,
            "decision_summary": proposal.decision_summary,
            "result_path": proposal.result_path,
            "result_commit": proposal.result_commit,
            "reviewed_at": proposal.reviewed_at,
        }),
    );
    Ok(proposal)
}

pub async fn get_for_bear(
    pool: &PgPool,
    bear_id: Uuid,
    proposal_id: Uuid,
) -> Result<Option<MemoryProposalRow>, CustomError> {
    let row = sqlx::query(
        r#"
        SELECT id, bear_id, source_role, source_agent_id, source_paths, source_refs,
               proposal_type, suggested_action, target_ref, title, summary, rationale,
               proposed_content, proposed_patch, refs, sensitivity, requires_human, status,
               reviewer_role, reviewer_agent_id, review_notes, decision_summary,
               result_path, result_commit, created_at, reviewed_at
        FROM bear_memory_proposals
        WHERE bear_id = $1 AND id = $2
        "#,
    )
    .bind(bear_id)
    .bind(proposal_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_from_sql))
}

fn row_from_sql(row: sqlx::postgres::PgRow) -> MemoryProposalRow {
    MemoryProposalRow {
        id: row.get("id"),
        bear_id: row.get("bear_id"),
        source_role: row.get("source_role"),
        source_agent_id: row.get("source_agent_id"),
        source_paths: row.get("source_paths"),
        source_refs: row.get("source_refs"),
        proposal_type: row.get("proposal_type"),
        suggested_action: row.get("suggested_action"),
        target_ref: row.get("target_ref"),
        title: row.get("title"),
        summary: row.get("summary"),
        rationale: row.get("rationale"),
        proposed_content: row.get("proposed_content"),
        proposed_patch: row.get("proposed_patch"),
        refs: row.get("refs"),
        sensitivity: row.get("sensitivity"),
        requires_human: row.get("requires_human"),
        status: row.get("status"),
        reviewer_role: row.get("reviewer_role"),
        reviewer_agent_id: row.get("reviewer_agent_id"),
        review_notes: row.get("review_notes"),
        decision_summary: row.get("decision_summary"),
        result_path: row.get("result_path"),
        result_commit: row.get("result_commit"),
        created_at: row.get("created_at"),
        reviewed_at: row.get("reviewed_at"),
    }
}
