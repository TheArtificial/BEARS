use den_core::{config::Config, DenError};
use den_memory::{
    harvest_source_hash_marked, harvest_source_marked, record_harvest_mark, MemoryStoreManager,
};
use den_service::memory_proposals::CreateMemoryProposal;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{memory::create_proposal, runtime_conversations::RuntimeIterativeSummary};
use den_service::bears::BearProfile;

#[derive(Debug, Clone, sqlx::FromRow)]
struct CompactionArtifactHarvestRow {
    id: Uuid,
    conversation_id: Uuid,
    external_conversation_id: Option<String>,
    artifact_kind: String,
    policy_version: String,
    trigger: String,
    source_message_start_seq: i64,
    source_message_end_seq: i64,
    source_group_start: Option<i32>,
    source_group_end: Option<i32>,
    artifact_json: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveHarvestOutput {
    pub scanned_artifacts: usize,
    pub created_proposal_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HarvestAssessment {
    durable_signal_count: usize,
    confidence: &'static str,
    sensitivity: &'static str,
    risk_signals: Vec<&'static str>,
    discarded_reasons: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HarvestExtraction {
    proposed_content: String,
    durable_signal_count: usize,
    confidence: &'static str,
    discarded_reasons: Vec<&'static str>,
}

fn db_err(context: &'static str) -> impl FnOnce(sqlx::Error) -> DenError {
    move |err| match DenError::from(err) {
        DenError::Database(message) => DenError::Database(format!("{context}: {message}")),
        DenError::DatabaseUnavailable(message) => {
            DenError::DatabaseUnavailable(format!("{context}: {message}"))
        }
        other => other,
    }
}

fn json_parse_err(context: &'static str) -> impl FnOnce(serde_json::Error) -> DenError {
    move |err| DenError::Parsing(format!("{context}: {err}"))
}

pub async fn harvest_compaction_artifacts_once(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    limit: i64,
    run_id: Option<&str>,
) -> Result<ArchiveHarvestOutput, DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    let artifacts = list_unmined_compaction_artifacts(pool, bear_id, limit).await?;
    let mut output = ArchiveHarvestOutput {
        scanned_artifacts: artifacts.len(),
        created_proposal_ids: Vec::new(),
    };

    for artifact in artifacts {
        let CompactionArtifactHarvestRow {
            id,
            conversation_id,
            external_conversation_id,
            artifact_kind,
            policy_version,
            trigger,
            source_message_start_seq,
            source_message_end_seq,
            source_group_start,
            source_group_end,
            artifact_json,
        } = artifact;

        let source_ref = id.to_string();
        let source_hash = source_hash(&artifact_json);
        // ponytail: exact content hash only; upgrade path is source equivalence metadata when
        // artifacts can be reserialized with semantically identical but byte-different JSON.
        if harvest_source_marked(&store, "compaction_artifact", &source_ref).await?
            || harvest_source_hash_marked(&store, "compaction_artifact", &source_hash).await?
        {
            continue;
        }
        let summary = decode_summary(artifact_json)?;
        let Some(extraction) = extract_harvest_candidate(&summary) else {
            record_harvest_mark(
                &store,
                "compaction_artifact",
                &source_ref,
                Some(&source_hash),
                run_id,
                &[],
            )
            .await?;
            continue;
        };
        let HarvestExtraction {
            proposed_content,
            durable_signal_count,
            confidence,
            discarded_reasons,
        } = extraction;
        let assessment = assess_harvest_candidate(
            &proposed_content,
            durable_signal_count,
            confidence,
            discarded_reasons,
        );

        let HarvestAssessment {
            durable_signal_count,
            confidence,
            sensitivity,
            risk_signals,
            discarded_reasons,
        } = assessment;
        let title = proposal_title(&summary, id);
        let rationale = format!(
            "Mined from Den compaction artifact {id} produced by policy {policy_version} from source messages {source_message_start_seq}-{source_message_end_seq}."
        );
        let source_refs = serde_json::json!({
            "source": "archive_harvest",
            "artifact_id": id,
            "artifact_kind": artifact_kind,
            "conversation_uuid": conversation_id,
            "conversation_id": &external_conversation_id,
            "policy_version": policy_version,
            "trigger": trigger,
            "source_message_start_seq": source_message_start_seq,
            "source_message_end_seq": source_message_end_seq,
            "source_group_start": source_group_start,
            "source_group_end": source_group_end,
        });
        let proposal = create_proposal(
            pool,
            config,
            stores,
            CreateMemoryProposal {
                bear_id,
                source_profile: BearProfile::Curate,
                source_agent_id: Some("archive_harvest".to_string()),
                source_paths: Vec::new(),
                source_refs,
                suggested_action: "human_review",
                target_ref: None,
                title: &title,
                summary: "Review compaction-derived session knowledge for possible durable memory promotion.",
                rationale: &rationale,
                proposed_content: Some(&proposed_content),
                proposed_patch: None,
                refs: serde_json::json!({
                    "conversation_id": &external_conversation_id,
                    "artifact_id": id,
                    "archive_harvest": true,
                    "source_hash": source_hash,
                    "quality": {
                        "confidence": confidence,
                        "durable_signal_count": durable_signal_count,
                        "detector": "archive-harvest-extraction-v2",
                        "adr": "ADR-0041",
                    },
                    "risk_signals": risk_signals,
                    "discarded_reasons": discarded_reasons,
                }),
                sensitivity,
                requires_human: true,
                project_to_conversation: false,
            },
        )
        .await?;
        output.created_proposal_ids.push(proposal.id);
        record_harvest_mark(
            &store,
            "compaction_artifact",
            &source_ref,
            Some(&source_hash),
            run_id,
            &[proposal.id.to_string()],
        )
        .await?;
    }

    Ok(output)
}

async fn list_unmined_compaction_artifacts(
    pool: &PgPool,
    bear_id: Uuid,
    limit: i64,
) -> Result<Vec<CompactionArtifactHarvestRow>, DenError> {
    sqlx::query_as::<_, CompactionArtifactHarvestRow>(
        r"
        SELECT a.id,
               a.conversation_id,
               c.external_conversation_id,
               a.artifact_kind,
               a.policy_version,
               a.trigger,
               a.source_message_start_seq,
               a.source_message_end_seq,
               a.source_group_start,
               a.source_group_end,
               a.artifact_json
        FROM conversation_compaction_artifacts a
        JOIN conversations c ON c.id = a.conversation_id
        WHERE c.bear_id = $1
          AND a.superseded_by IS NULL
          AND NOT EXISTS (
              SELECT 1
              FROM bear_memory_proposals p
              WHERE p.bear_id = $1
                AND p.source_refs->>'source' = 'archive_harvest'
                AND p.source_refs->>'artifact_id' = a.id::text
          )
        ORDER BY a.created_at ASC
        LIMIT $2
        ",
    )
    .bind(bear_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(db_err("list/decode compaction artifacts for harvest"))
}

fn decode_summary(value: serde_json::Value) -> Result<RuntimeIterativeSummary, DenError> {
    serde_json::from_value(value).map_err(json_parse_err("decode compaction summary for harvest"))
}

fn source_hash(value: &serde_json::Value) -> String {
    let encoded = serde_json::to_vec(value).unwrap_or_else(|_| value.to_string().into_bytes());
    let digest = Sha256::digest(encoded);
    format!("sha256:{digest:x}")
}

fn proposal_title(summary: &RuntimeIterativeSummary, artifact_id: Uuid) -> String {
    summary
        .important_constraints
        .iter()
        .chain(summary.decisions_made.iter())
        .chain(
            summary
                .active_user_goals
                .iter()
                .filter(|goal| looks_durable_goal(goal)),
        )
        .find_map(|value| {
            let text = value.trim();
            (!text.is_empty()).then(|| truncate_chars(text, 80))
        })
        .unwrap_or_else(|| format!("Review compaction artifact {artifact_id}"))
}

fn extract_harvest_candidate(summary: &RuntimeIterativeSummary) -> Option<HarvestExtraction> {
    let mut out = String::new();
    let mut discarded_reasons = Vec::new();

    let durable_goals = summary
        .active_user_goals
        .iter()
        .filter_map(|value| {
            let text = value.trim();
            if text.is_empty() {
                None
            } else if looks_durable_goal(text) {
                Some(text.to_string())
            } else {
                discarded_reasons.push("goal:transient_goal");
                None
            }
        })
        .collect::<Vec<_>>();

    append_section(&mut out, "Active user goals", &durable_goals);
    append_section(
        &mut out,
        "Important constraints",
        &summary.important_constraints,
    );
    append_section(&mut out, "Decisions made", &summary.decisions_made);

    discarded_reasons.extend(
        summary
            .artifact_refs
            .iter()
            .filter(|value| !value.trim().is_empty())
            .map(|_| "artifact:reference_without_semantic_claim"),
    );
    discarded_reasons.extend(
        summary
            .workflow_state_refs
            .iter()
            .filter(|value| !value.trim().is_empty())
            .map(|_| "workflow_state:transient_workflow_state"),
    );
    discarded_reasons.extend(
        summary
            .unresolved_followups
            .iter()
            .filter(|value| !value.trim().is_empty())
            .map(|_| "followup:transient_followup"),
    );
    discarded_reasons.sort_unstable();
    discarded_reasons.dedup();

    let durable_signal_count = durable_goals.len()
        + summary
            .important_constraints
            .iter()
            .filter(|value| !value.trim().is_empty())
            .count()
        + summary
            .decisions_made
            .iter()
            .filter(|value| !value.trim().is_empty())
            .count();
    if durable_signal_count == 0 {
        return None;
    }

    // ponytail: archive harvest trusts only compaction fields that already encode durable
    // semantic claims. Ceiling: artifact refs can point at valuable durable context; upgrade
    // path is source-turn-aware/model-assisted extraction instead of promoting references.
    let confidence = if summary
        .important_constraints
        .iter()
        .any(|value| !value.trim().is_empty())
        || summary
            .decisions_made
            .iter()
            .any(|value| !value.trim().is_empty())
    {
        "high"
    } else {
        "medium"
    };

    Some(HarvestExtraction {
        proposed_content: out.trim().to_string(),
        durable_signal_count,
        confidence,
        discarded_reasons,
    })
}

fn assess_harvest_candidate(
    proposed_content: &str,
    durable_signal_count: usize,
    confidence: &'static str,
    discarded_reasons: Vec<&'static str>,
) -> HarvestAssessment {
    let haystack = proposed_content.to_ascii_lowercase();
    let mut risk_signals = Vec::new();
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
        risk_signals.push("secret_risk");
    }
    if haystack.contains("http://")
        || haystack.contains("https://")
        || haystack.contains("external")
        || haystack.contains("untrusted")
    {
        risk_signals.push("external_untrusted");
    }
    if contains_any(
        &haystack,
        &["prefers", "preference", "human ", "user ", "personally"],
    ) {
        risk_signals.push("person");
    }
    risk_signals.sort_unstable();
    risk_signals.dedup();

    let sensitivity = if risk_signals.contains(&"secret_risk") {
        "secret_risk"
    } else if risk_signals.contains(&"external_untrusted") {
        "external_untrusted"
    } else if risk_signals.contains(&"person") {
        "person"
    } else {
        "normal"
    };

    HarvestAssessment {
        durable_signal_count,
        confidence,
        sensitivity,
        risk_signals,
        discarded_reasons,
    }
}

fn looks_durable_goal(text: &str) -> bool {
    let haystack = text.to_ascii_lowercase();
    contains_any(
        &haystack,
        &[
            "remember",
            "durable",
            "long-term",
            "long term",
            "ongoing",
            "always",
            "prefer",
            "preference",
            "policy",
        ],
    )
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn append_section(out: &mut String, title: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    out.push_str("## ");
    out.push_str(title);
    out.push('\n');
    for value in values {
        out.push_str("- ");
        out.push_str(value.trim());
        out.push('\n');
    }
    out.push('\n');
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

    fn summary_with(
        constraints: &[&str],
        decisions: &[&str],
        artifacts: &[&str],
        followups: &[&str],
    ) -> RuntimeIterativeSummary {
        RuntimeIterativeSummary {
            active_user_goals: Vec::new(),
            important_constraints: constraints
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            decisions_made: decisions.iter().map(|value| (*value).to_string()).collect(),
            artifact_refs: artifacts.iter().map(|value| (*value).to_string()).collect(),
            workflow_state_refs: Vec::new(),
            unresolved_followups: followups.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    #[test]
    fn extraction_renders_only_durable_summary_sections() {
        let summary = RuntimeIterativeSummary {
            active_user_goals: vec![
                "ship compaction".to_string(),
                "remember the long-term curation policy".to_string(),
            ],
            important_constraints: vec!["do not cross approval floors".to_string()],
            decisions_made: Vec::new(),
            artifact_refs: vec![
                "docs/roadmap/DEN_CONTEXT_COMPACTION_IMPLEMENTATION_PLAN.md".to_string()
            ],
            workflow_state_refs: Vec::new(),
            unresolved_followups: vec!["wire archive harvest".to_string()],
        };

        let extraction = extract_harvest_candidate(&summary).expect("extraction");
        let rendered = extraction.proposed_content;

        assert!(rendered.contains("## Active user goals"));
        assert!(rendered.contains("remember the long-term curation policy"));
        assert!(rendered.contains("do not cross approval floors"));
        assert!(!rendered.contains("ship compaction"));
        assert!(!rendered.contains("wire archive harvest"));
        assert!(!rendered.contains("DEN_CONTEXT_COMPACTION_IMPLEMENTATION_PLAN"));
        assert!(extraction
            .discarded_reasons
            .contains(&"artifact:reference_without_semantic_claim"));
        assert!(extraction
            .discarded_reasons
            .contains(&"followup:transient_followup"));
        assert!(extraction
            .discarded_reasons
            .contains(&"goal:transient_goal"));
    }

    #[test]
    fn extraction_drops_transient_followups_only() {
        let summary = summary_with(&[], &[], &[], &["remember to rerun tests"]);

        assert!(extract_harvest_candidate(&summary).is_none());
    }

    #[test]
    fn extraction_keeps_durable_decisions_with_high_confidence() {
        let summary = summary_with(
            &["Do not auto-promote raw transcripts."],
            &["Use SQLite as canonical memory."],
            &[],
            &[],
        );
        let extraction = extract_harvest_candidate(&summary).expect("extraction");
        let assessment = assess_harvest_candidate(
            &extraction.proposed_content,
            extraction.durable_signal_count,
            extraction.confidence,
            extraction.discarded_reasons,
        );

        assert_eq!(assessment.confidence, "high");
        assert_eq!(assessment.sensitivity, "normal");
        assert_eq!(assessment.durable_signal_count, 2);
    }

    #[test]
    fn extraction_drops_artifact_references_without_semantic_claims() {
        let summary = summary_with(
            &[],
            &[],
            &["docs/roadmap/MEMORY_AUTOMATION_ROADMAP.md"],
            &[],
        );

        assert!(extract_harvest_candidate(&summary).is_none());
    }

    #[test]
    fn extraction_drops_goals_and_workflow_state_without_durable_signals() {
        let mut summary = summary_with(&[], &[], &[], &["revisit later"]);
        summary.active_user_goals = vec!["finish the current task".to_string()];
        summary.workflow_state_refs = vec!["job-123".to_string()];

        assert!(extract_harvest_candidate(&summary).is_none());
    }

    #[test]
    fn assessment_flags_secret_external_and_person_risk() {
        let summary = summary_with(
            &["Hans prefers not to share the API key from https://example.invalid."],
            &[],
            &[],
            &[],
        );
        let extraction = extract_harvest_candidate(&summary).expect("extraction");
        let assessment = assess_harvest_candidate(
            &extraction.proposed_content,
            extraction.durable_signal_count,
            extraction.confidence,
            extraction.discarded_reasons,
        );

        assert_eq!(assessment.sensitivity, "secret_risk");
        assert!(assessment.risk_signals.contains(&"secret_risk"));
        assert!(assessment.risk_signals.contains(&"external_untrusted"));
        assert!(assessment.risk_signals.contains(&"person"));
    }
}
