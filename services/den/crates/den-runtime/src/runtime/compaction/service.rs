use den_core::{config::Config, profile::BearProfile, DenError};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    agent_loop::load_transcript_grouping_rows,
    runtime_compaction_observability::{
        build_compaction_applied_event, build_compaction_skipped_event, RuntimeCompactionEvent,
        RuntimeCompactionEventStatus,
    },
    runtime_compaction_store::record_runtime_compaction_event,
    runtime_conversations::{RuntimeCompactionTriggerKind, RuntimeSemanticGroup},
};

use super::{
    artifact_store,
    policy::{compaction_policy_for_profile, CompactionMode},
    render::render_compacted_context_block,
    summarize::summarize_compacted_groups,
    {
        artifact_ref_from_decision, choose_compaction_decision,
        RuntimeCompactionDecision, RuntimeCompactionPolicy,
    },
};

/// When compaction evaluation runs during turn assembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCompactionTrigger {
    TurnStart,
    Manual,
}

impl TurnCompactionTrigger {
    fn as_runtime_trigger(self) -> RuntimeCompactionTriggerKind {
        match self {
            Self::TurnStart => RuntimeCompactionTriggerKind::SemanticGroupCount,
            Self::Manual => RuntimeCompactionTriggerKind::Manual,
        }
    }
}

/// Compaction state produced for a single turn assembly.
#[derive(Debug, Clone)]
pub struct TurnCompactionState {
    pub event: RuntimeCompactionEvent,
    pub decision: Option<RuntimeCompactionDecision>,
    pub policy: RuntimeCompactionPolicy,
    pub groups: Vec<RuntimeSemanticGroup>,
    pub compacted_seq_cutoff: Option<i64>,
    pub compacted_context: Option<String>,
}

pub async fn prepare_turn_compaction(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
    conversation_id: &str,
    profile: BearProfile,
    trigger: TurnCompactionTrigger,
) -> Result<Option<TurnCompactionState>, DenError> {
    let mode = CompactionMode::parse(&config.compaction_mode);
    if mode == CompactionMode::Off {
        return Ok(None);
    }

    let rows = load_transcript_grouping_rows(pool, bear_id, conversation_id).await?;
    let groups = super::semantic_groups_from_conversation_messages(&rows);
    let policy = compaction_policy_for_profile(profile);
    let runtime_trigger = trigger.as_runtime_trigger();

    let prior_summary = if mode == CompactionMode::Active {
        artifact_store::load_latest_iterative_summary(pool, bear_id, conversation_id).await?
    } else {
        None
    };

    let mut decision = choose_compaction_decision(&groups, runtime_trigger.clone(), &policy);
    if decision.is_none() && mode == CompactionMode::Active {
        let transcript_chars = estimate_transcript_chars(&rows);
        if transcript_chars > policy.max_transcript_chars {
            decision = choose_compaction_decision(
                &groups,
                RuntimeCompactionTriggerKind::TokenPressure,
                &policy,
            );
        }
    }

    let (event, compacted_seq_cutoff, compacted_context) = match &decision {
        Some(decision) => {
            let artifact_id = format!(
                "{conversation_id}:{}-{}",
                decision.selected_group_start, decision.selected_group_end
            );
            let artifact_ref =
                artifact_ref_from_decision(artifact_id, decision, &policy);
            let event = build_compaction_applied_event(
                conversation_id.to_string(),
                decision,
                &policy,
                artifact_ref,
            );

            let (cutoff, context) = if mode == CompactionMode::Active {
                let compacted_groups =
                    &groups[decision.selected_group_start..=decision.selected_group_end];
                let summary = summarize_compacted_groups(
                    prior_summary.as_ref().map(|record| &record.summary),
                    compacted_groups,
                    &rows,
                    decision,
                );
                let (start_seq, end_seq) =
                    sequence_span_for_group_range(&rows, &groups, decision)?;
                let _artifact_uuid = artifact_store::insert_iterative_summary_artifact(
                    pool,
                    bear_id,
                    conversation_id,
                    decision,
                    &policy,
                    runtime_trigger.clone(),
                    start_seq,
                    end_seq,
                    &summary,
                )
                .await?;
                let cutoff = Some(end_seq);
                let context = Some(render_compacted_context_block(&summary));
                (cutoff, context)
            } else {
                (None, None)
            };

            (event, cutoff, context)
        }
        None => {
            let diagnostic = if groups.is_empty() {
                "no transcript groups to evaluate"
            } else {
                "no eligible history groups outside protected floors"
            };
            let event = build_compaction_skipped_event(
                conversation_id.to_string(),
                runtime_trigger,
                &policy,
                diagnostic,
            );
            let context = prior_summary
                .as_ref()
                .map(|record| render_compacted_context_block(&record.summary));
            (event, prior_summary.map(|record| record.source_message_end_seq), context)
        }
    };

    record_runtime_compaction_event(pool, &event).await?;

    Ok(Some(TurnCompactionState {
        event,
        decision,
        policy,
        groups,
        compacted_seq_cutoff,
        compacted_context,
    }))
}

pub fn render_compaction_prompt_context(state: &TurnCompactionState) -> String {
    let compacted = state.compacted_context.as_deref().unwrap_or_default();
    let recent_count = state
        .policy
        .protected_recent_group_count
        .min(state.groups.len());
    let status = match state.event.status {
        RuntimeCompactionEventStatus::Applied => "applied",
        RuntimeCompactionEventStatus::Skipped => "skipped",
        RuntimeCompactionEventStatus::Failed => "failed",
    };

    let mut parts = vec![format!(
        "Runtime compaction context is Den-owned. Treat active instructions, workflow state, recent uncompacted groups, and compacted summary state as distinct context layers. Current compaction evaluation: status={status} policy_version={} source_range={:?}-{:?} diagnostic={}. Recent protected groups retained: {recent_count}.",
        state.policy.policy_version,
        state.event.source_group_start,
        state.event.source_group_end,
        state.event.diagnostic.as_deref().unwrap_or("none"),
    )];

    if !compacted.is_empty() {
        parts.push(compacted.to_string());
    }

    parts.join("\n\n")
}

fn estimate_transcript_chars(rows: &[super::TranscriptGroupingRow]) -> usize {
    rows.iter()
        .map(|row| row.content_text.chars().count())
        .sum()
}

fn sequence_span_for_group_range(
    rows: &[super::TranscriptGroupingRow],
    groups: &[RuntimeSemanticGroup],
    decision: &RuntimeCompactionDecision,
) -> Result<(i64, i64), DenError> {
    let start_group = groups
        .get(decision.selected_group_start)
        .ok_or_else(|| DenError::System("compaction group start out of range".into()))?;
    let end_group = groups
        .get(decision.selected_group_end)
        .ok_or_else(|| DenError::System("compaction group end out of range".into()))?;

    let start_seq = message_sequence(rows, start_group.start_message_id.as_deref())
        .ok_or_else(|| DenError::System("compaction start message missing sequence".into()))?;
    let end_seq = message_sequence(rows, end_group.end_message_id.as_deref())
        .or_else(|| message_sequence(rows, end_group.start_message_id.as_deref()))
        .ok_or_else(|| DenError::System("compaction end message missing sequence".into()))?;
    Ok((start_seq, end_seq))
}

fn message_sequence(rows: &[super::TranscriptGroupingRow], message_id: Option<&str>) -> Option<i64> {
    let message_id = message_id?;
    rows.iter()
        .find(|row| row.message_id.as_deref() == Some(message_id))
        .and_then(|row| row.sequence_no)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_conversations::RuntimeSemanticGroupKind;

    #[test]
    fn estimate_transcript_chars_sums_row_text() {
        let rows = vec![
            super::super::TranscriptGroupingRow::new("user", "hello", serde_json::json!({})),
            super::super::TranscriptGroupingRow::new("assistant", "world!", serde_json::json!({})),
        ];
        assert_eq!(estimate_transcript_chars(&rows), 11);
    }

    #[test]
    fn render_compaction_prompt_context_includes_status_and_summary() {
        let state = TurnCompactionState {
            event: build_compaction_skipped_event(
                "conv-1",
                RuntimeCompactionTriggerKind::SemanticGroupCount,
                &compaction_policy_for_profile(BearProfile::Pair),
                "test",
            ),
            decision: None,
            policy: compaction_policy_for_profile(BearProfile::Pair),
            groups: vec![RuntimeSemanticGroup {
                kind: RuntimeSemanticGroupKind::UserTurn,
                start_message_id: None,
                end_message_id: None,
                message_count: 1,
                protected: false,
            }],
            compacted_seq_cutoff: None,
            compacted_context: Some("# Compacted context\n\n- goal: ship".into()),
        };
        let text = render_compaction_prompt_context(&state);
        assert!(text.contains("status=skipped"));
        assert!(text.contains("# Compacted context"));
    }
}
