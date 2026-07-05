use den_core::{config::Config, DenError};
use den_memory::{harvest_source_marked, record_harvest_mark, MemoryStoreManager};
use den_service::memory_proposals::CreateMemoryProposal;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{memory::create_proposal, runtime_conversations::RuntimeIterativeSummary};
use den_service::bears::BearProfile;

#[derive(Debug, Clone)]
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

pub async fn harvest_compaction_artifacts_once(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    bear_id: Uuid,
    limit: i64,
) -> Result<ArchiveHarvestOutput, DenError> {
    let store = stores.store_for_bear(bear_id).await?;
    let artifacts = list_unmined_compaction_artifacts(pool, bear_id, limit).await?;
    let mut output = ArchiveHarvestOutput {
        scanned_artifacts: artifacts.len(),
        created_proposal_ids: Vec::new(),
    };

    for artifact in artifacts {
        let source_ref = artifact.id.to_string();
        if harvest_source_marked(&store, "compaction_artifact", &source_ref).await? {
            continue;
        }
        let summary = decode_summary(&artifact.artifact_json)?;
        let proposed_content = proposal_content_from_summary(&summary);
        if proposed_content.trim().is_empty() {
            record_harvest_mark(&store, "compaction_artifact", &source_ref, None, None, &[])
                .await?;
            continue;
        }

        let title = proposal_title(&summary, &artifact);
        let rationale = format!(
            "Mined from Den compaction artifact {} produced by policy {} from source messages {}-{}.",
            artifact.id,
            artifact.policy_version,
            artifact.source_message_start_seq,
            artifact.source_message_end_seq
        );
        let source_refs = serde_json::json!({
            "source": "archive_harvest",
            "artifact_id": artifact.id,
            "artifact_kind": artifact.artifact_kind,
            "conversation_uuid": artifact.conversation_id,
            "conversation_id": artifact.external_conversation_id,
            "policy_version": artifact.policy_version,
            "trigger": artifact.trigger,
            "source_message_start_seq": artifact.source_message_start_seq,
            "source_message_end_seq": artifact.source_message_end_seq,
            "source_group_start": artifact.source_group_start,
            "source_group_end": artifact.source_group_end,
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
                suggested_action: "review",
                target_ref: None,
                title: &title,
                summary: "Review compaction-derived session knowledge for possible durable memory promotion.",
                rationale: &rationale,
                proposed_content: Some(&proposed_content),
                proposed_patch: None,
                refs: serde_json::json!({
                    "conversation_id": artifact.external_conversation_id,
                    "artifact_id": artifact.id,
                }),
                sensitivity: "normal",
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
            None,
            None,
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
    let rows = sqlx::query(
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
    .map_err(|err| DenError::Database(format!("list compaction artifacts for harvest: {err}")))?;

    rows.into_iter()
        .map(|row| {
            Ok(CompactionArtifactHarvestRow {
                id: row.try_get("id")?,
                conversation_id: row.try_get("conversation_id")?,
                external_conversation_id: row.try_get("external_conversation_id")?,
                artifact_kind: row.try_get("artifact_kind")?,
                policy_version: row.try_get("policy_version")?,
                trigger: row.try_get("trigger")?,
                source_message_start_seq: row.try_get("source_message_start_seq")?,
                source_message_end_seq: row.try_get("source_message_end_seq")?,
                source_group_start: row.try_get("source_group_start")?,
                source_group_end: row.try_get("source_group_end")?,
                artifact_json: row.try_get("artifact_json")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|err| DenError::Database(format!("decode compaction artifact harvest row: {err}")))
}

fn decode_summary(value: &serde_json::Value) -> Result<RuntimeIterativeSummary, DenError> {
    serde_json::from_value(value.clone())
        .map_err(|err| DenError::Database(format!("decode compaction summary for harvest: {err}")))
}

fn proposal_title(
    summary: &RuntimeIterativeSummary,
    artifact: &CompactionArtifactHarvestRow,
) -> String {
    summary
        .active_user_goals
        .first()
        .map(|goal| truncate_chars(goal, 80))
        .unwrap_or_else(|| format!("Review compaction artifact {}", artifact.id))
}

fn proposal_content_from_summary(summary: &RuntimeIterativeSummary) -> String {
    let mut out = String::new();
    append_section(&mut out, "Active user goals", &summary.active_user_goals);
    append_section(
        &mut out,
        "Important constraints",
        &summary.important_constraints,
    );
    append_section(&mut out, "Decisions made", &summary.decisions_made);
    append_section(&mut out, "Artifact references", &summary.artifact_refs);
    append_section(
        &mut out,
        "Workflow state references",
        &summary.workflow_state_refs,
    );
    append_section(
        &mut out,
        "Unresolved follow-ups",
        &summary.unresolved_followups,
    );
    out.trim().to_string()
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

    #[test]
    fn proposal_content_renders_structured_summary_sections() {
        let summary = RuntimeIterativeSummary {
            active_user_goals: vec!["ship compaction".to_string()],
            important_constraints: vec!["do not cross approval floors".to_string()],
            decisions_made: Vec::new(),
            artifact_refs: vec![
                "docs/roadmap/DEN_CONTEXT_COMPACTION_IMPLEMENTATION_PLAN.md".to_string()
            ],
            workflow_state_refs: Vec::new(),
            unresolved_followups: vec!["wire archive harvest".to_string()],
        };

        let rendered = proposal_content_from_summary(&summary);

        assert!(rendered.contains("## Active user goals"));
        assert!(rendered.contains("ship compaction"));
        assert!(rendered.contains("do not cross approval floors"));
        assert!(rendered.contains("wire archive harvest"));
    }
}
