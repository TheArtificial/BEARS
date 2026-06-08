use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        bears::BearAgentRole,
        memory_manager_head::{write_memfs_core_update, MemfsCoreUpdateRequest},
        memory_proposals::{self, MemoryProposalRow, ProposalResolutionParams},
        tools::memfs::memfs_http_client,
    },
    errors::CustomError,
};

pub const MEMORY_CURATE_RUNNER_AGENT_ID: &str = "memory_curate_runner";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurateProposalOutcome {
    pub proposal_id: Uuid,
    pub status: String,
    pub suggested_action: String,
    pub triage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCurateRunOutput {
    pub resolved_proposal_ids: Vec<String>,
    pub outcomes: Vec<CurateProposalOutcome>,
    pub resolution_status: String,
    pub status_counts: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CurateTriage {
    RetainRoleLocal {
        review_notes: &'static str,
        decision_summary: &'static str,
    },
    Reject {
        review_notes: &'static str,
        decision_summary: &'static str,
    },
    Defer {
        review_notes: &'static str,
        decision_summary: &'static str,
    },
    EscalateHuman {
        review_notes: &'static str,
        decision_summary: &'static str,
    },
    PromoteToCore {
        review_notes: &'static str,
        decision_summary: &'static str,
    },
}

impl CurateTriage {
    fn triage_label(&self) -> &'static str {
        match self {
            Self::RetainRoleLocal { .. } => "retain_role_local",
            Self::Reject { .. } => "reject",
            Self::Defer { .. } => "defer",
            Self::EscalateHuman { .. } => "escalate_human",
            Self::PromoteToCore { .. } => "promote_to_core",
        }
    }

    fn resolution_status(&self) -> &'static str {
        match self {
            Self::RetainRoleLocal { .. } => "retained_local",
            Self::Reject { .. } => "rejected",
            Self::Defer { .. } => "deferred",
            Self::EscalateHuman { .. } => "needs_human_review",
            Self::PromoteToCore { .. } => "approved",
        }
    }

    fn review_notes(&self) -> &'static str {
        match self {
            Self::RetainRoleLocal { review_notes, .. }
            | Self::Reject { review_notes, .. }
            | Self::Defer { review_notes, .. }
            | Self::EscalateHuman { review_notes, .. }
            | Self::PromoteToCore { review_notes, .. } => review_notes,
        }
    }

    fn decision_summary(&self) -> &'static str {
        match self {
            Self::RetainRoleLocal { decision_summary, .. }
            | Self::Reject { decision_summary, .. }
            | Self::Defer { decision_summary, .. }
            | Self::EscalateHuman { decision_summary, .. }
            | Self::PromoteToCore { decision_summary, .. } => decision_summary,
        }
    }
}

fn decide_curate_triage(proposal: &MemoryProposalRow, trigger: Option<&str>) -> CurateTriage {
    if proposal.requires_human {
        return CurateTriage::EscalateHuman {
            review_notes: "Proposal was flagged requires_human=true.",
            decision_summary: "Escalated to human review because the proposal requires human follow-up.",
        };
    }

    if sensitivity_requires_human(&proposal.sensitivity) {
        return CurateTriage::EscalateHuman {
            review_notes: "Proposal sensitivity requires human review.",
            decision_summary: "Escalated to human review because sensitivity policy blocks autonomous resolution.",
        };
    }

    match proposal.suggested_action.as_str() {
        "human_review" => CurateTriage::EscalateHuman {
            review_notes: "Proposal requested explicit human review.",
            decision_summary: "Escalated to human review per suggested_action=human_review.",
        },
        "retain_role_local" => CurateTriage::RetainRoleLocal {
            review_notes: "Role-local memory remains the durable source; no shared-memory write needed.",
            decision_summary: "Autonomous curate retained the proposal as role-local memory.",
        },
        "delete_after_review" => CurateTriage::Reject {
            review_notes: "Proposal requested deletion after review.",
            decision_summary: "Autonomous curate rejected the proposal per delete_after_review.",
        },
        "promote_to_core" | "summarize_into_core" => {
            if can_auto_promote_to_core(proposal) {
                CurateTriage::PromoteToCore {
                    review_notes: "Low-risk core promotion candidate with bounded summary content.",
                    decision_summary: "Autonomous curate applied a distilled summary to core memory.",
                }
            } else {
                CurateTriage::Defer {
                    review_notes: "Core promotion requires curate-agent review or additional proposal content.",
                    decision_summary: "Deferred core promotion until curate can review with richer context.",
                }
            }
        }
        "cabinet_update" | "skill_review" | "archive_index" | "task_context" => CurateTriage::Defer {
            review_notes: "Specialized promotion path is not handled by the autonomous substrate yet.",
            decision_summary: "Deferred until curate can route the proposal through the appropriate workflow.",
        },
        "unspecified" => {
            if trigger == Some("pair_reflection") && proposal.sensitivity == "normal" {
                CurateTriage::RetainRoleLocal {
                    review_notes: "Pair reflection summary is durable in pair-local memory; shared promotion is not automatic.",
                    decision_summary: "Autonomous curate retained the pair reflection summary as role-local memory.",
                }
            } else {
                CurateTriage::Defer {
                    review_notes: "Ambiguous proposal needs curate-agent review.",
                    decision_summary: "Deferred unspecified proposal until curate can decide the final outcome.",
                }
            }
        }
        _ => CurateTriage::Defer {
            review_notes: "Unknown suggested_action; deferring to curate review.",
            decision_summary: "Deferred proposal with unrecognized suggested_action.",
        },
    }
}

fn sensitivity_requires_human(sensitivity: &str) -> bool {
    matches!(
        sensitivity,
        "person" | "secret_risk" | "external_untrusted" | "unknown"
    )
}

fn can_auto_promote_to_core(proposal: &MemoryProposalRow) -> bool {
    if proposal.sensitivity != "normal" {
        return false;
    }
    let has_body = proposal
        .proposed_content
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || proposal.summary.trim().len() >= 16;
    has_body && !proposal.source_paths.is_empty()
}

fn core_target_path(proposal: &MemoryProposalRow) -> String {
    proposal
        .target_ref
        .as_deref()
        .map(str::trim)
        .filter(|path| path.starts_with("core/"))
        .map(str::to_string)
        .unwrap_or_else(|| "core/knowledge.md".to_string())
}

fn promotion_body(proposal: &MemoryProposalRow) -> String {
    let distilled = proposal
        .proposed_content
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| proposal.summary.trim());
    format!(
        "{distilled}\n\n---\nSource proposal: `{}`\nSource role: `{}`\nSource paths: {}\n",
        proposal.id,
        proposal.source_role,
        proposal.source_paths.join(", ")
    )
}

pub async fn execute_memory_curate_proposals(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
    trigger: Option<&str>,
    proposal_ids: &[Uuid],
) -> Result<MemoryCurateRunOutput, CustomError> {
    let mut outcomes = Vec::new();
    for proposal_id in proposal_ids {
        let Some(proposal) = memory_proposals::get_for_bear(pool, bear_id, *proposal_id).await?
        else {
            continue;
        };
        if proposal.status != "pending" {
            continue;
        }
        let outcome = resolve_curate_proposal(pool, config, bear_id, &proposal, trigger).await?;
        outcomes.push(outcome);
    }

    let resolved_proposal_ids = outcomes
        .iter()
        .map(|outcome| outcome.proposal_id.to_string())
        .collect::<Vec<_>>();
    let mut status_counts = serde_json::Map::new();
    for outcome in &outcomes {
        let count = status_counts
            .entry(outcome.status.clone())
            .or_insert(serde_json::Value::from(0));
        if let Some(number) = count.as_i64() {
            *count = serde_json::Value::from(number + 1);
        }
    }
    let resolution_status = aggregate_resolution_status(&outcomes);

    Ok(MemoryCurateRunOutput {
        resolved_proposal_ids,
        outcomes,
        resolution_status,
        status_counts: serde_json::Value::Object(status_counts),
    })
}

async fn resolve_curate_proposal(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
    proposal: &MemoryProposalRow,
    trigger: Option<&str>,
) -> Result<CurateProposalOutcome, CustomError> {
    let triage = decide_curate_triage(proposal, trigger);
    let triage_label = triage.triage_label().to_string();

    if matches!(triage, CurateTriage::PromoteToCore { .. }) {
        match apply_core_promotion(pool, config, bear_id, proposal, &triage).await {
            Ok((result_path, _result_commit)) => {
                return Ok(CurateProposalOutcome {
                    proposal_id: proposal.id,
                    status: "approved".to_string(),
                    suggested_action: proposal.suggested_action.clone(),
                    triage: triage_label,
                    result_path: Some(result_path),
                    error: None,
                });
            }
            Err(error) => {
                let deferred = memory_proposals::resolve_for_bear(
                    pool,
                    ProposalResolutionParams {
                        bear_id,
                        proposal_id: proposal.id,
                        reviewer_role: BearAgentRole::Curate,
                        reviewer_agent_id: Some(MEMORY_CURATE_RUNNER_AGENT_ID),
                        status: "deferred",
                        review_notes: Some(&format!(
                            "Autonomous core promotion failed; deferred for curate review: {error}"
                        )),
                        decision_summary: Some(
                            "Deferred after autonomous core promotion attempt failed.",
                        ),
                        result_path: None,
                        result_commit: None,
                        project_to_conversation: true,
                    },
                )
                .await?;
                return Ok(CurateProposalOutcome {
                    proposal_id: deferred.id,
                    status: deferred.status,
                    suggested_action: deferred.suggested_action,
                    triage: "defer_after_error".to_string(),
                    result_path: None,
                    error: Some(error.to_string()),
                });
            }
        }
    }

    let resolved = memory_proposals::resolve_for_bear(
        pool,
        ProposalResolutionParams {
            bear_id,
            proposal_id: proposal.id,
            reviewer_role: BearAgentRole::Curate,
            reviewer_agent_id: Some(MEMORY_CURATE_RUNNER_AGENT_ID),
            status: triage.resolution_status(),
            review_notes: Some(triage.review_notes()),
            decision_summary: Some(triage.decision_summary()),
            result_path: None,
            result_commit: None,
            project_to_conversation: true,
        },
    )
    .await?;

    Ok(CurateProposalOutcome {
        proposal_id: resolved.id,
        status: resolved.status,
        suggested_action: resolved.suggested_action,
        triage: triage_label,
        result_path: resolved.result_path,
        error: None,
    })
}

async fn apply_core_promotion(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
    proposal: &MemoryProposalRow,
    triage: &CurateTriage,
) -> Result<(String, Option<String>), CustomError> {
    if config.letta_memfs_service_url.trim().is_empty() {
        return Err(CustomError::System(
            "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)".to_string(),
        ));
    }
    let http = memfs_http_client("MemFS memory_curate core update client build failed")?;
    let request = MemfsCoreUpdateRequest {
        target_path: core_target_path(proposal),
        mode: "append_section".to_string(),
        title: Some(proposal.title.clone()),
        body: Some(promotion_body(proposal)),
        old_text: None,
        new_text: None,
        proposal_id: Some(proposal.id),
        source_paths: proposal.source_paths.clone(),
    };
    let response = write_memfs_core_update(
        &http,
        &config.letta_memfs_service_url,
        bear_id,
        &request,
    )
    .await?;
    let Some(response) = response else {
        return Err(CustomError::System(
            "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)".to_string(),
        ));
    };
    memory_proposals::resolve_for_bear(
        pool,
        ProposalResolutionParams {
            bear_id,
            proposal_id: proposal.id,
            reviewer_role: BearAgentRole::Curate,
            reviewer_agent_id: Some(MEMORY_CURATE_RUNNER_AGENT_ID),
            status: "approved",
            review_notes: Some(triage.review_notes()),
            decision_summary: Some(triage.decision_summary()),
            result_path: Some(response.path.as_str()),
            result_commit: response.canonical_tip.as_deref(),
            project_to_conversation: true,
        },
    )
    .await?;
    Ok((response.path, response.canonical_tip))
}

fn aggregate_resolution_status(outcomes: &[CurateProposalOutcome]) -> String {
    if outcomes.is_empty() {
        return "no_pending_proposals".to_string();
    }
    let mut distinct = outcomes
        .iter()
        .map(|outcome| outcome.status.as_str())
        .collect::<Vec<_>>();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() == 1 {
        distinct[0].to_string()
    } else {
        "mixed".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    fn sample_proposal(suggested_action: &str, sensitivity: &str, requires_human: bool) -> MemoryProposalRow {
        MemoryProposalRow {
            id: Uuid::new_v4(),
            bear_id: Uuid::new_v4(),
            source_role: "pair".to_string(),
            source_agent_id: Some("pair-agent".to_string()),
            source_paths: vec!["pair/summaries/example.md".to_string()],
            source_refs: serde_json::json!({}),
            proposal_type: "memory_review".to_string(),
            suggested_action: suggested_action.to_string(),
            target_ref: None,
            title: "Test proposal".to_string(),
            summary: "Useful durable lesson from the pair session.".to_string(),
            rationale: "rationale".to_string(),
            proposed_content: None,
            proposed_patch: None,
            refs: serde_json::json!({}),
            sensitivity: sensitivity.to_string(),
            requires_human,
            status: "pending".to_string(),
            reviewer_role: None,
            reviewer_agent_id: None,
            review_notes: None,
            decision_summary: None,
            result_path: None,
            result_commit: None,
            created_at: OffsetDateTime::now_utc(),
            reviewed_at: None,
        }
    }

    #[test]
    fn pair_reflection_unspecified_normal_is_retained_local() {
        let proposal = sample_proposal("unspecified", "normal", false);
        let triage = decide_curate_triage(&proposal, Some("pair_reflection"));
        assert_eq!(triage.resolution_status(), "retained_local");
    }

    #[test]
    fn unspecified_without_pair_reflection_is_deferred() {
        let proposal = sample_proposal("unspecified", "normal", false);
        let triage = decide_curate_triage(&proposal, Some("manual"));
        assert_eq!(triage.resolution_status(), "deferred");
    }

    #[test]
    fn requires_human_escalates() {
        let proposal = sample_proposal("retain_role_local", "normal", true);
        let triage = decide_curate_triage(&proposal, Some("pair_reflection"));
        assert_eq!(triage.resolution_status(), "needs_human_review");
    }

    #[test]
    fn promote_to_core_with_summary_is_promotable() {
        let proposal = sample_proposal("promote_to_core", "normal", false);
        let triage = decide_curate_triage(&proposal, None);
        assert_eq!(triage.triage_label(), "promote_to_core");
    }

    #[test]
    fn cabinet_update_is_deferred() {
        let proposal = sample_proposal("cabinet_update", "normal", false);
        let triage = decide_curate_triage(&proposal, None);
        assert_eq!(triage.resolution_status(), "deferred");
    }

    #[test]
    fn aggregate_resolution_status_reports_mixed_outcomes() {
        let status = aggregate_resolution_status(&[
            CurateProposalOutcome {
                proposal_id: Uuid::new_v4(),
                status: "retained_local".to_string(),
                suggested_action: "unspecified".to_string(),
                triage: "retain_role_local".to_string(),
                result_path: None,
                error: None,
            },
            CurateProposalOutcome {
                proposal_id: Uuid::new_v4(),
                status: "deferred".to_string(),
                suggested_action: "cabinet_update".to_string(),
                triage: "defer".to_string(),
                result_path: None,
                error: None,
            },
        ]);
        assert_eq!(status, "mixed");
    }
}
