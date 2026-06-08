use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        bears::BearAgentRole,
        conversation_events::{
            memory_proposal_resolved_projection, memory_review_requested_projection,
            project_to_conversation, ProjectionProvenance, ProjectionSource,
        },
        memory::{
            create_proposal, get_proposal, list_proposals, promote_core_content, resolve_proposal,
            MemoryStoreManager,
        },
        memory_manager_head::MemfsCoreUpdateRequest,
        memory_proposals::{CreateMemoryProposal, ProposalResolutionParams},
        tools::{
            memfs::memfs_http_client,
            memory_write::source_acp_session_id,
            session::DenToolInvocationContext,
            support::{clean_optional, validate_bounded_text, validate_optional_object},
        },
    },
    errors::CustomError,
};

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryListProposalsArguments {
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryReadProposalArguments {
    pub(crate) proposal_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryApplyCoreUpdateArguments {
    pub(crate) proposal_id: Uuid,
    pub(crate) target_path: String,
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) body: Option<String>,
    #[serde(default)]
    pub(crate) old_text: Option<String>,
    #[serde(default)]
    pub(crate) new_text: Option<String>,
    #[serde(default)]
    pub(crate) review_notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryResolveProposalArguments {
    pub(crate) proposal_id: Uuid,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) review_notes: Option<String>,
    #[serde(default)]
    pub(crate) decision_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryRequestReviewArguments {
    pub(crate) source_paths: Vec<String>,
    pub(crate) title: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) rationale: String,
    #[serde(default)]
    pub(crate) suggested_action: Option<String>,
    #[serde(default)]
    pub(crate) target_ref: Option<String>,
    #[serde(default)]
    pub(crate) refs: Option<Value>,
    #[serde(default)]
    pub(crate) sensitivity: Option<String>,
    #[serde(default)]
    pub(crate) requires_human: bool,
    #[serde(default)]
    pub(crate) proposed_content: Option<String>,
    #[serde(default)]
    pub(crate) proposed_patch: Option<String>,
}

pub(crate) async fn apply_core_update(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Curate {
        return Err(CustomError::Authorization(
            "den.memory.apply_core_update is available only to curate".to_string(),
        ));
    }
    let args: MemoryApplyCoreUpdateArguments = serde_json::from_value(arguments)?;
    let proposal = get_proposal(pool, config, stores, context.bear_id, args.proposal_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("memory proposal not found".to_string()))?;
    if config.uses_native_agent_runtime() {
        let content = args.body.unwrap_or_else(|| {
            format!(
                "Applied from proposal `{}` via native SQLite promotion.",
                proposal.id
            )
        });
        let kind = args
            .target_path
            .split('/')
            .next_back()
            .unwrap_or("note")
            .trim_end_matches(".md");
        let (memory_id, promotion_id) = promote_core_content(
            stores,
            context.bear_id,
            &proposal.id.to_string(),
            kind,
            &content,
            role.as_str(),
        )
        .await?;
        let resolved = resolve_proposal(
            pool,
            config,
            stores,
            ProposalResolutionParams {
                bear_id: context.bear_id,
                proposal_id: proposal.id,
                reviewer_role: role,
                reviewer_agent_id: Some(context.role_agent_id.as_str()),
                status: "approved",
                review_notes: args.review_notes.as_deref(),
                decision_summary: Some("Applied reviewed memory proposal to core (SQLite)."),
                result_path: Some(args.target_path.as_str()),
                result_commit: None,
                project_to_conversation: false,
            },
        )
        .await?;
        project_to_conversation(
            pool,
            context.bear_id,
            Some(context.user_id),
            clean_optional(&context.conversation_id).as_deref(),
            memory_proposal_resolved_projection(
                ProjectionProvenance {
                    source: ProjectionSource::DenTools,
                    scope_id: source_acp_session_id(context)
                        .or_else(|| clean_optional(&context.session_id))
                        .unwrap_or_else(|| format!("bear:{}:role:{}", context.bear_id, role.as_str())),
                },
                resolved.id,
                resolved.source_role.clone(),
                resolved.suggested_action.clone(),
                resolved.title.clone(),
                resolved.status.clone(),
                resolved.reviewer_role.clone(),
                resolved.result_path.clone(),
                resolved.result_commit.clone(),
            ),
        );
        return Ok(json!({
            "bear_id": context.bear_id,
            "proposal": resolved,
            "core_update": {
                "path": args.target_path,
                "memory_id": memory_id,
                "promotion_id": promotion_id,
            },
        }));
    }
    let http = memfs_http_client("MemFS core update client build failed")?;
    let body = args.body.map(|body| {
        format!(
            "{}\n\n---\nSource proposal: `{}`\nSource role: `{}`\nSource paths: {}\n",
            body.trim(),
            proposal.id,
            proposal.source_role,
            proposal.source_paths.join(", ")
        )
    });
    let request = MemfsCoreUpdateRequest {
        target_path: args.target_path,
        mode: args.mode,
        title: args.title.or(Some(proposal.title.clone())),
        body,
        old_text: args.old_text,
        new_text: args.new_text,
        proposal_id: Some(proposal.id),
        source_paths: proposal.source_paths.clone(),
    };
    let response = crate::core::memory_manager_head::write_memfs_core_update(
        &http,
        &config.letta_memfs_service_url,
        context.bear_id,
        &request,
    )
    .await?;
    let Some(response) = response else {
        return Err(CustomError::System(
            "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)".to_string(),
        ));
    };
    let resolved = resolve_proposal(
        pool,
        config,
        stores,
        ProposalResolutionParams {
            bear_id: context.bear_id,
            proposal_id: proposal.id,
            reviewer_role: role,
            reviewer_agent_id: Some(context.role_agent_id.as_str()),
            status: "approved",
            review_notes: args.review_notes.as_deref(),
            decision_summary: Some("Applied reviewed memory proposal to core."),
            result_path: Some(response.path.as_str()),
            result_commit: response.canonical_tip.as_deref(),
            project_to_conversation: false,
        },
    )
    .await?;
    project_to_conversation(
        pool,
        context.bear_id,
        Some(context.user_id),
        clean_optional(&context.conversation_id).as_deref(),
        memory_proposal_resolved_projection(
            ProjectionProvenance {
                source: ProjectionSource::DenTools,
                scope_id: source_acp_session_id(context)
                    .or_else(|| clean_optional(&context.session_id))
                    .unwrap_or_else(|| format!("bear:{}:role:{}", context.bear_id, role.as_str())),
            },
            resolved.id,
            resolved.source_role.clone(),
            resolved.suggested_action.clone(),
            resolved.title.clone(),
            resolved.status.clone(),
            resolved.reviewer_role.clone(),
            resolved.result_path.clone(),
            resolved.result_commit.clone(),
        ),
    );
    Ok(json!({
        "bear_id": context.bear_id,
        "proposal": resolved,
        "core_update": response,
    }))
}

pub(crate) async fn list_memory_proposals(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Curate {
        return Err(CustomError::Authorization(
            "den.memory.list_proposals is available only to curate".to_string(),
        ));
    }
    let args: MemoryListProposalsArguments = serde_json::from_value(arguments)?;
    let status = args.status.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let proposals =
        list_proposals(pool, config, stores, context.bear_id, status, args.limit.unwrap_or(50))
            .await?;
    Ok(json!({ "bear_id": context.bear_id, "proposals": proposals }))
}

pub(crate) async fn read_memory_proposal(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Curate {
        return Err(CustomError::Authorization(
            "den.memory.read_proposal is available only to curate".to_string(),
        ));
    }
    let args: MemoryReadProposalArguments = serde_json::from_value(arguments)?;
    let proposal = get_proposal(pool, config, stores, context.bear_id, args.proposal_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("memory proposal not found".to_string()))?;
    Ok(json!({ "bear_id": context.bear_id, "proposal": proposal }))
}

pub(crate) async fn resolve_memory_proposal(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Curate {
        return Err(CustomError::Authorization(
            "den.memory.resolve_proposal is available only to curate".to_string(),
        ));
    }
    let args: MemoryResolveProposalArguments = serde_json::from_value(arguments)?;
    let status = args.status.trim();
    if !matches!(
        status,
        "rejected" | "retained_local" | "deferred" | "superseded" | "needs_human_review"
    ) {
        return Err(CustomError::ValidationError(
            "status must be rejected, retained_local, deferred, superseded, or needs_human_review"
                .to_string(),
        ));
    }
    let proposal = resolve_proposal(
        pool,
        config,
        stores,
        ProposalResolutionParams {
            bear_id: context.bear_id,
            proposal_id: args.proposal_id,
            reviewer_role: role,
            reviewer_agent_id: Some(context.role_agent_id.as_str()),
            status,
            review_notes: args.review_notes.as_deref(),
            decision_summary: args.decision_summary.as_deref(),
            result_path: None,
            result_commit: None,
            project_to_conversation: false,
        },
    )
    .await?;
    project_to_conversation(
        pool,
        context.bear_id,
        Some(context.user_id),
        clean_optional(&context.conversation_id).as_deref(),
        memory_proposal_resolved_projection(
            ProjectionProvenance {
                source: ProjectionSource::DenTools,
                scope_id: source_acp_session_id(context)
                    .or_else(|| clean_optional(&context.session_id))
                    .unwrap_or_else(|| format!("bear:{}:role:{}", context.bear_id, role.as_str())),
            },
            proposal.id,
            proposal.source_role.clone(),
            proposal.suggested_action.clone(),
            proposal.title.clone(),
            proposal.status.clone(),
            proposal.reviewer_role.clone(),
            proposal.result_path.clone(),
            proposal.result_commit.clone(),
        ),
    );
    Ok(json!({ "bear_id": context.bear_id, "proposal": proposal }))
}

pub(crate) async fn request_memory_review(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if !matches!(role, BearAgentRole::Pair) {
        return Err(CustomError::Authorization(
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
        return Err(CustomError::ValidationError(
            "source_paths must include at least one path".to_string(),
        ));
    }
    if source_paths.len() > 20 {
        return Err(CustomError::ValidationError(
            "source_paths must include at most 20 paths".to_string(),
        ));
    }
    for path in &source_paths {
        if !path.starts_with(role.as_str()) || !path.ends_with(".md") {
            return Err(CustomError::ValidationError(format!(
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
        .unwrap_or("unspecified");
    let sensitivity = args
        .sensitivity
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("normal");
    let proposal = create_proposal(
        pool,
        config,
        stores,
        CreateMemoryProposal {
            bear_id: context.bear_id,
            source_role: role,
            source_agent_id: clean_optional(&context.role_agent_id),
            source_paths,
            source_refs: serde_json::json!({
                "conversation_id": clean_optional(&context.conversation_id),
                "session_id": source_acp_session_id(context).or_else(|| clean_optional(&context.session_id)),
                "request_id": context.request_id,
                "runtime_target": context.runtime_target,
            }),
            suggested_action,
            target_ref: args
                .target_ref
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            title: &title,
            summary: &summary,
            rationale: &rationale,
            proposed_content: args.proposed_content.as_deref(),
            proposed_patch: args.proposed_patch.as_deref(),
            refs: args.refs.unwrap_or_else(|| serde_json::json!({})),
            sensitivity,
            requires_human: args.requires_human,
            project_to_conversation: false,
        },
    )
    .await?;
    project_to_conversation(
        pool,
        context.bear_id,
        Some(context.user_id),
        clean_optional(&context.conversation_id).as_deref(),
        memory_review_requested_projection(
            ProjectionProvenance {
                source: ProjectionSource::DenTools,
                scope_id: source_acp_session_id(context)
                    .or_else(|| clean_optional(&context.session_id))
                    .unwrap_or_else(|| format!("bear:{}:role:{}", context.bear_id, role.as_str())),
            },
            proposal.id,
            proposal.source_role.clone(),
            proposal.suggested_action.clone(),
            proposal.title.clone(),
            proposal.status.clone(),
            proposal.source_paths.clone(),
        ),
    );
    Ok(json!({
        "bear_id": context.bear_id,
        "proposal": proposal,
        "note": "Review requested. Reflection/curate decides the final outcome; this did not write core, Cabinet, skills, tasks, observations, or run results."
    }))
}
