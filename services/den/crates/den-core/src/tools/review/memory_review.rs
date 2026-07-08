//! Memory review/curation tools — orchestration layer.
//!
//! `list_proposals` / `read_proposal` / `resolve_proposal` / `request_review` /
//! `apply_core_update`. Runtime-agnostic: role gating, argument validation, and
//! projection-scope computation over the [`MemoryReviewStore`] seam; the `den`
//! impl owns the capability calls and `conversation_events` projections.

use crate::{BearProfile, DenError};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::tools::{
    context::DenToolInvocationContext,
    memory::source_client_session_id,
    support::{clean_optional, validate_bounded_text, validate_optional_object},
};

use super::store::{
    ApplyCoreUpdateRequest, MarkMemoryLifecycleRequest, MemoryReviewStore, ProposalProjection,
    RequestReviewRequest, ResolveProposalRequest,
};

#[derive(Debug, Deserialize)]
pub struct MemoryListProposalsArguments {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryReadProposalArguments {
    pub proposal_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct MemoryApplyCoreUpdateArguments {
    pub proposal_id: Uuid,
    pub target_path: String,
    pub mode: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub old_text: Option<String>,
    #[serde(default)]
    pub new_text: Option<String>,
    #[serde(default)]
    pub review_notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryResolveProposalArguments {
    pub proposal_id: Uuid,
    pub status: String,
    #[serde(default)]
    pub review_notes: Option<String>,
    #[serde(default)]
    pub decision_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryMarkLifecycleArguments {
    pub memory_id: String,
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryRequestReviewArguments {
    pub source_paths: Vec<String>,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub suggested_action: Option<String>,
    #[serde(default)]
    pub target_ref: Option<String>,
    #[serde(default)]
    pub refs: Option<Value>,
    #[serde(default)]
    pub sensitivity: Option<String>,
    #[serde(default)]
    pub requires_human: bool,
    #[serde(default)]
    pub proposed_content: Option<String>,
    #[serde(default)]
    pub proposed_patch: Option<String>,
}

/// The `conversation_events` projection scope id for this invocation.
fn projection_scope_id(
    context: &DenToolInvocationContext,
    bear_id: Uuid,
    role: BearProfile,
) -> String {
    source_client_session_id(context)
        .or_else(|| clean_optional(&context.session_id))
        .unwrap_or_else(|| format!("bear:{}:role:{}", bear_id, role.as_str()))
}

fn projection(
    context: &DenToolInvocationContext,
    bear_id: Uuid,
    role: BearProfile,
) -> ProposalProjection {
    ProposalProjection {
        user_id: context.user_id,
        conversation_id: clean_optional(&context.conversation_id),
        scope_id: projection_scope_id(context, bear_id, role),
    }
}

pub async fn apply_core_update(
    store: &impl MemoryReviewStore,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Curate {
        return Err(DenError::Authorization(
            "den.memory.apply_core_update is available only to curate".to_string(),
        ));
    }
    let args: MemoryApplyCoreUpdateArguments = serde_json::from_value(arguments)?;
    store
        .apply_core_update(ApplyCoreUpdateRequest {
            bear_id: context.bear_id,
            reviewer_profile: role,
            binding_id: context.binding_id.clone(),
            proposal_id: args.proposal_id,
            target_path: args.target_path,
            mode: args.mode,
            title: args.title,
            body: args.body,
            old_text: args.old_text,
            new_text: args.new_text,
            review_notes: args.review_notes,
            projection: projection(context, context.bear_id, role),
        })
        .await
}

pub async fn mark_memory_lifecycle(
    store: &impl MemoryReviewStore,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Curate {
        return Err(DenError::Authorization(
            "den.memory.mark_lifecycle is available only to curate".to_string(),
        ));
    }
    let args: MemoryMarkLifecycleArguments = serde_json::from_value(arguments)?;
    let memory_id = validate_bounded_text("memory_id", &args.memory_id, 1, 200)?;
    let status = args.status.trim();
    if !matches!(
        status,
        "active" | "stale" | "superseded" | "archived" | "archive-candidate"
    ) {
        return Err(DenError::ValidationError(
            "status must be active, stale, superseded, archived, or archive-candidate".to_string(),
        ));
    }
    let reason = args
        .reason
        .as_deref()
        .map(|value| validate_bounded_text("reason", value, 0, 1_000))
        .transpose()?;
    let record = store
        .mark_memory_lifecycle(MarkMemoryLifecycleRequest {
            bear_id: context.bear_id,
            reviewer_profile: role,
            binding_id: context.binding_id.clone(),
            memory_id,
            status: status.to_string(),
            reason,
        })
        .await?;
    Ok(json!({ "bear_id": context.bear_id, "record": record }))
}

pub async fn list_memory_proposals(
    store: &impl MemoryReviewStore,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Curate {
        return Err(DenError::Authorization(
            "den.memory.list_proposals is available only to curate".to_string(),
        ));
    }
    let args: MemoryListProposalsArguments = serde_json::from_value(arguments)?;
    let status = args
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let proposals = store
        .list_proposals(context.bear_id, status, args.limit.unwrap_or(50))
        .await?;
    Ok(json!({ "bear_id": context.bear_id, "proposals": proposals }))
}

pub async fn read_memory_proposal(
    store: &impl MemoryReviewStore,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Curate {
        return Err(DenError::Authorization(
            "den.memory.read_proposal is available only to curate".to_string(),
        ));
    }
    let args: MemoryReadProposalArguments = serde_json::from_value(arguments)?;
    let proposal = store
        .get_proposal(context.bear_id, args.proposal_id)
        .await?
        .ok_or_else(|| DenError::NotFound("memory proposal not found".to_string()))?;
    Ok(json!({ "bear_id": context.bear_id, "proposal": proposal }))
}

pub async fn resolve_memory_proposal(
    store: &impl MemoryReviewStore,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Curate {
        return Err(DenError::Authorization(
            "den.memory.resolve_proposal is available only to curate".to_string(),
        ));
    }
    let args: MemoryResolveProposalArguments = serde_json::from_value(arguments)?;
    let status = args.status.trim();
    if !matches!(
        status,
        "rejected" | "retained_local" | "deferred" | "superseded" | "needs_human_review"
    ) {
        return Err(DenError::ValidationError(
            "status must be rejected, retained_local, deferred, superseded, or needs_human_review"
                .to_string(),
        ));
    }
    let proposal = store
        .resolve_proposal(ResolveProposalRequest {
            bear_id: context.bear_id,
            reviewer_profile: role,
            binding_id: context.binding_id.clone(),
            proposal_id: args.proposal_id,
            status: status.to_string(),
            review_notes: args.review_notes,
            decision_summary: args.decision_summary,
            projection: projection(context, context.bear_id, role),
        })
        .await?;
    Ok(json!({ "bear_id": context.bear_id, "proposal": proposal }))
}

// `.md` is the canonical, case-sensitive role-memory extension; keep the exact
// original comparison rather than clippy's case-insensitive suggestion.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub async fn request_memory_review(
    store: &impl MemoryReviewStore,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if !matches!(role, BearProfile::Pair) {
        return Err(DenError::Authorization(
            "den.memory.request_review is currently available only to pair".to_string(),
        ));
    }
    let args: MemoryRequestReviewArguments = serde_json::from_value(arguments)?;
    let source_paths = args
        .source_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    if source_paths.is_empty() {
        return Err(DenError::ValidationError(
            "source_paths must include at least one path".to_string(),
        ));
    }
    if source_paths.len() > 20 {
        return Err(DenError::ValidationError(
            "source_paths must include at most 20 paths".to_string(),
        ));
    }
    for path in &source_paths {
        if !path.starts_with(role.as_str()) || !path.ends_with(".md") {
            return Err(DenError::ValidationError(format!(
                "source path must be a role-local Markdown path under {}/: {path}",
                role.as_str()
            )));
        }
    }
    let title = validate_bounded_text("title", &args.title, 1, 200)?;
    let summary = validate_bounded_text("summary", &args.summary, 1, 4_000)?;
    let rationale = validate_bounded_text("rationale", &args.rationale, 0, 4_000)?;
    validate_optional_object("refs", &args.refs)?;
    let suggested_action = args
        .suggested_action
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("unspecified")
        .to_string();
    let sensitivity = args
        .sensitivity
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("normal")
        .to_string();
    let source_refs = json!({
        "conversation_id": clean_optional(&context.conversation_id),
        "session_id": source_client_session_id(context).or_else(|| clean_optional(&context.session_id)),
        "request_id": context.request_id,
        "runtime_target": context.runtime_target,
    });
    let proposal = store
        .request_review(RequestReviewRequest {
            bear_id: context.bear_id,
            source_profile: role,
            binding_id: clean_optional(&context.binding_id),
            source_paths,
            source_refs,
            suggested_action,
            target_ref: args
                .target_ref
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            title,
            summary,
            rationale,
            proposed_content: args.proposed_content,
            proposed_patch: args.proposed_patch,
            refs: args.refs.unwrap_or_else(|| json!({})),
            sensitivity,
            requires_human: args.requires_human,
            projection: projection(context, context.bear_id, role),
        })
        .await?;
    Ok(json!({
        "bear_id": context.bear_id,
        "proposal": proposal,
        "note": "Review requested. Reflection/curate decides the final outcome; this did not write core, Cabinet, skills, tasks, observations, or run results."
    }))
}
