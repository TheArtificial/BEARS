use den_core::{config::Config, DenError};
use den_memory::MemoryStoreManager;
use den_service::{bears::BearProfile, memory_proposals::CreateMemoryProposal};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    memory::create_proposal,
    reflection::conductor::{enqueue_memory_curate_for_proposals, ProposalEnqueueParams},
    runtime::compaction::artifact_store::{
        load_latest_iterative_summary, CompactionArtifactRecord,
    },
    runtime_conversations::RuntimeIterativeSummary,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairReflectionProposalOutput {
    pub created_proposal_ids: Vec<Uuid>,
    pub candidate_count: usize,
    pub dropped_followup_count: usize,
    pub skipped_reason: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PairReflectionCandidate {
    kind: &'static str,
    text: String,
    title: String,
    suggested_action: &'static str,
    target_ref: Option<&'static str>,
    sensitivity: &'static str,
    requires_human: bool,
}

pub async fn create_pair_reflection_proposals_from_latest_summary(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    conversation_id: &str,
    session_id: &str,
) -> Result<PairReflectionProposalOutput, DenError> {
    let Some(artifact) = load_latest_iterative_summary(pool, bear_id, conversation_id).await?
    else {
        return Ok(PairReflectionProposalOutput {
            skipped_reason: Some("no_compaction_artifact"),
            ..PairReflectionProposalOutput::default()
        });
    };
    create_pair_reflection_proposals_for_artifact(
        pool,
        config,
        stores,
        bear_id,
        conversation_id,
        session_id,
        &artifact,
    )
    .await
}

async fn create_pair_reflection_proposals_for_artifact(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    conversation_id: &str,
    session_id: &str,
    artifact: &CompactionArtifactRecord,
) -> Result<PairReflectionProposalOutput, DenError> {
    let candidates = candidates_from_summary(&artifact.summary);
    let mut output = PairReflectionProposalOutput {
        candidate_count: candidates.len(),
        dropped_followup_count: artifact.summary.unresolved_followups.len(),
        skipped_reason: None,
        created_proposal_ids: Vec::new(),
    };

    for candidate in candidates {
        let source_refs = serde_json::json!({
            "source": "pair_reflection",
            "conversation_id": conversation_id,
            "session_id": session_id,
            "compaction_artifact_id": artifact.artifact_id,
            "source_message_start_seq": artifact.source_message_start_seq,
            "source_message_end_seq": artifact.source_message_end_seq,
            "policy_version": artifact.policy_version,
            "candidate_kind": candidate.kind,
        });
        let refs = serde_json::json!({
            "pair_reflection": true,
            "candidate_kind": candidate.kind,
            "quality": {
                "confidence": if candidate.kind == "artifact" { "medium" } else { "high" },
                "detector": "pair-reflection-summary-v1"
            }
        });
        let rationale = format!(
            "Extracted from pair reflection compaction artifact {} for conversation {}.",
            artifact.artifact_id, conversation_id
        );
        let proposal = create_proposal(
            pool,
            config,
            stores,
            CreateMemoryProposal {
                bear_id,
                source_profile: BearProfile::Pair,
                source_agent_id: Some("pair_reflection".to_string()),
                source_paths: Vec::new(),
                source_refs,
                suggested_action: candidate.suggested_action,
                target_ref: candidate.target_ref,
                title: &candidate.title,
                summary: "Review pair reflection candidate for durable memory.",
                rationale: &rationale,
                proposed_content: Some(&candidate.text),
                proposed_patch: None,
                refs,
                sensitivity: candidate.sensitivity,
                requires_human: candidate.requires_human,
                project_to_conversation: false,
            },
        )
        .await?;
        output.created_proposal_ids.push(proposal.id);
    }

    if !output.created_proposal_ids.is_empty() {
        let _ = enqueue_memory_curate_for_proposals(
            pool,
            ProposalEnqueueParams {
                bear_id,
                binding_id: Some("pair_reflection"),
                conversation_id: Some(conversation_id),
                conversation_key: Some(conversation_id),
                conversation_date: None,
                trigger: "pair_reflection",
                proposal_ids: output.created_proposal_ids.clone(),
            },
        )
        .await?;
    }

    Ok(output)
}

fn candidates_from_summary(summary: &RuntimeIterativeSummary) -> Vec<PairReflectionCandidate> {
    let mut candidates = Vec::new();
    for value in &summary.decisions_made {
        push_candidate(
            &mut candidates,
            "decision",
            value,
            "Pair reflection decision",
        );
    }
    for value in &summary.important_constraints {
        push_candidate(
            &mut candidates,
            "constraint",
            value,
            "Pair reflection constraint",
        );
    }
    for value in &summary.artifact_refs {
        push_candidate(
            &mut candidates,
            "artifact",
            value,
            "Pair reflection artifact",
        );
    }
    for value in &summary.active_user_goals {
        push_candidate(
            &mut candidates,
            "goal",
            value,
            "Pair reflection goal",
        );
    }
    for value in &summary.workflow_state_refs {
        push_candidate(
            &mut candidates,
            "workflow_state",
            value,
            "Pair reflection workflow state",
        );
    }
    for value in &summary.unresolved_followups {
        push_candidate(
            &mut candidates,
            "followup",
            value,
            "Pair reflection follow-up",
        );
    }
    // ponytail: pair reflection v1 promotes every non-empty summary bucket to a proposal,
    // with transient buckets routed to human review. Ceiling: noisy summaries create noisy
    // proposals; upgrade path is model-assisted scoring with source turn refs.
    candidates
}

fn push_candidate(
    candidates: &mut Vec<PairReflectionCandidate>,
    kind: &'static str,
    value: &str,
    title_prefix: &'static str,
) {
    let text = value.trim();
    if text.is_empty() {
        return;
    }
    let sensitivity = sensitivity_for_text(text);
    candidates.push(PairReflectionCandidate {
        kind,
        text: text.to_string(),
        title: format!("{title_prefix}: {}", truncate_chars(text, 80)),
        suggested_action: if kind == "constraint" || kind == "decision" {
            "retain_profile_local"
        } else {
            "human_review"
        },
        target_ref: None,
        sensitivity,
        requires_human: sensitivity != "normal",
    });
}

fn sensitivity_for_text(text: &str) -> &'static str {
    let haystack = text.to_ascii_lowercase();
    if contains_any(
        &haystack,
        &[
            "api key",
            "password",
            "secret",
            "credential",
            "private key",
            "bearer ",
            "access token",
        ],
    ) {
        "secret_risk"
    } else if haystack.contains("http://")
        || haystack.contains("https://")
        || haystack.contains("external")
        || haystack.contains("untrusted")
    {
        "external_untrusted"
    } else if contains_any(
        &haystack,
        &["prefers", "preference", "human ", "user ", "personally"],
    ) {
        "person"
    } else {
        "normal"
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out = text.chars().take(max.saturating_sub(1)).collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> RuntimeIterativeSummary {
        RuntimeIterativeSummary {
            active_user_goals: vec!["finish the task".to_string()],
            important_constraints: vec!["Use proposal review before shared promotion.".to_string()],
            decisions_made: vec!["Keep pair reflection local by default.".to_string()],
            artifact_refs: vec!["docs/roadmap/MEMORY_CURATION_PLAN.md".to_string()],
            workflow_state_refs: vec!["job-123".to_string()],
            unresolved_followups: vec!["rerun tests".to_string()],
        }
    }

    #[test]
    fn candidates_include_all_non_empty_summary_buckets() {
        let candidates = candidates_from_summary(&summary());

        assert_eq!(candidates.len(), 6);
        assert!(candidates.iter().any(|c| c.kind == "decision"));
        assert!(candidates.iter().any(|c| c.kind == "constraint"));
        assert!(candidates.iter().any(|c| c.kind == "artifact"));
        assert!(candidates.iter().any(|c| c.kind == "goal"));
        assert!(candidates.iter().any(|c| c.kind == "workflow_state"));
        assert!(candidates.iter().any(|c| c.kind == "followup"));
    }

    #[test]
    fn candidates_route_pair_decisions_to_profile_local_review() {
        let candidates = candidates_from_summary(&summary());
        let decision = candidates.iter().find(|c| c.kind == "decision").unwrap();

        assert_eq!(decision.suggested_action, "retain_profile_local");
        assert_eq!(decision.sensitivity, "normal");
        assert!(!decision.requires_human);
    }

    #[test]
    fn candidates_force_human_review_for_risky_person_content() {
        let summary = RuntimeIterativeSummary {
            decisions_made: vec!["The user prefers not to share the API key.".to_string()],
            ..RuntimeIterativeSummary::default()
        };
        let candidates = candidates_from_summary(&summary);

        assert_eq!(candidates[0].sensitivity, "secret_risk");
        assert!(candidates[0].requires_human);
    }
}
