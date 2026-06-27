use crate::runtime_conversations::RuntimeSemanticGroup;
use den_core::DenError;

use super::{
    merge_iterative_summary, TranscriptGroupingRow, RuntimeCompactionDecision,
    RuntimeIterativeSummary,
};

/// Deterministic v1 summarization: merge structured labels from compacted groups.
///
/// LLM-backed `agent_compaction` can replace the label extraction later; this path is
/// intentionally fail-open and cheap for rollout.
pub fn summarize_compacted_groups(
    prior: Option<&RuntimeIterativeSummary>,
    compacted_groups: &[RuntimeSemanticGroup],
    rows: &[TranscriptGroupingRow],
    decision: &RuntimeCompactionDecision,
) -> RuntimeIterativeSummary {
    deterministic_summary(prior, compacted_groups, rows, decision)
}

#[derive(Debug, Clone)]
pub struct CompactionSummaryInput<'a> {
    pub prior: Option<&'a RuntimeIterativeSummary>,
    pub compacted_groups: &'a [RuntimeSemanticGroup],
    pub rows: &'a [TranscriptGroupingRow],
    pub decision: &'a RuntimeCompactionDecision,
}

pub trait CompactionSummarizer {
    fn summarize(&self, input: CompactionSummaryInput<'_>) -> Result<RuntimeIterativeSummary, DenError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicCompactionSummarizer;

impl CompactionSummarizer for DeterministicCompactionSummarizer {
    fn summarize(&self, input: CompactionSummaryInput<'_>) -> Result<RuntimeIterativeSummary, DenError> {
        Ok(deterministic_summary(
            input.prior,
            input.compacted_groups,
            input.rows,
            input.decision,
        ))
    }
}

/// Adapter contract for future `agent_compaction` model-task implementations.
///
/// Implementations may call an LLM, but they must return the same v1 structured
/// artifact shape. Validation happens here so compaction can fail open to the
/// deterministic summarizer instead of blocking prompt shrink.
pub struct LlmBackedCompactionSummarizer<F>
where
    F: Fn(CompactionSummaryPrompt) -> Result<RuntimeIterativeSummary, DenError>,
{
    generate: F,
}

#[derive(Debug, Clone)]
pub struct CompactionSummaryPrompt {
    pub system: String,
    pub user: String,
}

impl<F> LlmBackedCompactionSummarizer<F>
where
    F: Fn(CompactionSummaryPrompt) -> Result<RuntimeIterativeSummary, DenError>,
{
    pub fn new(generate: F) -> Self {
        Self { generate }
    }
}

impl<F> CompactionSummarizer for LlmBackedCompactionSummarizer<F>
where
    F: Fn(CompactionSummaryPrompt) -> Result<RuntimeIterativeSummary, DenError>,
{
    fn summarize(&self, input: CompactionSummaryInput<'_>) -> Result<RuntimeIterativeSummary, DenError> {
        let fallback = deterministic_summary(input.prior, input.compacted_groups, input.rows, input.decision);
        let prompt = build_compaction_summary_prompt(&fallback, &input);
        let generated = (self.generate)(prompt)?;
        Ok(validate_generated_summary(generated, fallback))
    }
}

fn deterministic_summary(
    prior: Option<&RuntimeIterativeSummary>,
    compacted_groups: &[RuntimeSemanticGroup],
    rows: &[TranscriptGroupingRow],
    decision: &RuntimeCompactionDecision,
) -> RuntimeIterativeSummary {
    let _ = decision;
    let mut summary = merge_iterative_summary(prior, compacted_groups);
    enrich_summary_from_rows(&mut summary, rows, compacted_groups);
    summary
}

fn build_compaction_summary_prompt(
    deterministic: &RuntimeIterativeSummary,
    input: &CompactionSummaryInput<'_>,
) -> CompactionSummaryPrompt {
    let compacted_rows = rows_for_compacted_groups(input.rows, input.compacted_groups)
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "sequence_no": row.sequence_no,
                "message_type": row.message_type,
                "content_text": row.content_text,
                "content_json": row.content_json,
            })
        })
        .collect::<Vec<_>>();
    let user = serde_json::json!({
        "task": "Produce a Den compaction iterative summary artifact. Preserve active goals, constraints, decisions, artifact references, workflow state, unresolved follow-ups, and provenance. Return only JSON matching the existing artifact shape.",
        "selected_group_start": input.decision.selected_group_start,
        "selected_group_end": input.decision.selected_group_end,
        "prior_summary": input.prior,
        "deterministic_baseline": deterministic,
        "compacted_rows": compacted_rows,
    });
    CompactionSummaryPrompt {
        system: "You are Den's agent_compaction summarizer. Optimize for continuation fidelity, not prose. Do not promote durable memory. Do not invent facts.".to_string(),
        user: serde_json::to_string_pretty(&user).unwrap_or_else(|_| user.to_string()),
    }
}

fn validate_generated_summary(
    mut generated: RuntimeIterativeSummary,
    fallback: RuntimeIterativeSummary,
) -> RuntimeIterativeSummary {
    ensure_fallback_items(&mut generated.active_user_goals, &fallback.active_user_goals);
    ensure_fallback_items(&mut generated.important_constraints, &fallback.important_constraints);
    ensure_fallback_items(&mut generated.decisions_made, &fallback.decisions_made);
    ensure_fallback_items(&mut generated.artifact_refs, &fallback.artifact_refs);
    ensure_fallback_items(&mut generated.workflow_state_refs, &fallback.workflow_state_refs);
    ensure_fallback_items(&mut generated.unresolved_followups, &fallback.unresolved_followups);
    generated
}

fn ensure_fallback_items(values: &mut Vec<String>, fallback: &[String]) {
    for item in fallback {
        push_unique(values, item.clone());
    }
}

fn enrich_summary_from_rows(
    summary: &mut RuntimeIterativeSummary,
    rows: &[TranscriptGroupingRow],
    compacted_groups: &[RuntimeSemanticGroup],
) {
    let compacted_rows = rows_for_compacted_groups(rows, compacted_groups);
    for row in compacted_rows {
        let snippet = compact_row_snippet(row);
        if snippet.is_empty() {
            continue;
        }
        match row.message_type.as_str() {
            "user" => push_unique(&mut summary.active_user_goals, snippet),
            "assistant" => push_unique(&mut summary.unresolved_followups, snippet),
            "tool_call" | "tool_result" => push_unique(&mut summary.artifact_refs, snippet),
            "workflow_event" => push_unique(&mut summary.workflow_state_refs, snippet),
            "compaction_marker" => {
                push_unique(&mut summary.important_constraints, snippet)
            }
            _ => push_unique(&mut summary.important_constraints, snippet),
        }
    }
}

fn rows_for_compacted_groups<'a>(
    rows: &'a [TranscriptGroupingRow],
    compacted_groups: &[RuntimeSemanticGroup],
) -> Vec<&'a TranscriptGroupingRow> {
    use std::collections::HashSet;

    let mut ids = HashSet::new();
    for group in compacted_groups {
        if let Some(id) = group.start_message_id.as_deref() {
            ids.insert(id);
        }
        if let Some(id) = group.end_message_id.as_deref() {
            ids.insert(id);
        }
    }
    rows.iter()
        .filter(|row| {
            row.message_id
                .as_deref()
                .is_some_and(|id| ids.contains(id))
        })
        .collect()
}

fn compact_row_snippet(row: &TranscriptGroupingRow) -> String {
    let text = row.content_text.trim();
    if !text.is_empty() {
        return truncate_chars(text, 160);
    }
    row.content_json
        .get("tool_name")
        .and_then(|v| v.as_str())
        .map(|name| format!("tool:{name}"))
        .unwrap_or_default()
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if value.is_empty() {
        return;
    }
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_compacted_groups_adds_row_snippets() {
        let rows = vec![
            TranscriptGroupingRow::new("user", "please compact this session", json!({}))
                .with_message_id("msg-1")
                .with_sequence_no(1),
            TranscriptGroupingRow::new("assistant", "acknowledged", json!({}))
                .with_message_id("msg-2")
                .with_sequence_no(2),
        ];
        let groups = super::super::semantic_groups_from_conversation_messages(&rows);
        let decision = RuntimeCompactionDecision {
            trigger: crate::runtime_conversations::RuntimeCompactionTriggerKind::Manual,
            strategy: super::super::RuntimeCompactionStrategy::UpdateIterativeSummary,
            boundary: crate::runtime_conversations::RuntimeCompactionBoundary {
                retained_group_count: 0,
                compacted_group_count: groups.len(),
            },
            selected_group_start: 0,
            selected_group_end: groups.len().saturating_sub(1),
        };

        let summary = summarize_compacted_groups(None, &groups, &rows, &decision);
        assert!(summary
            .active_user_goals
            .iter()
            .any(|v| v.contains("please compact")));
        assert!(summary
            .unresolved_followups
            .iter()
            .any(|v| v.contains("acknowledged")));
    }

    #[test]
    fn llm_backed_summarizer_keeps_required_deterministic_fallback_items() {
        let rows = vec![
            TranscriptGroupingRow::new("user", "keep artifact docs/plan.md", json!({}))
                .with_message_id("msg-1")
                .with_sequence_no(1),
        ];
        let groups = super::super::semantic_groups_from_conversation_messages(&rows);
        let decision = RuntimeCompactionDecision {
            trigger: crate::runtime_conversations::RuntimeCompactionTriggerKind::Manual,
            strategy: super::super::RuntimeCompactionStrategy::UpdateIterativeSummary,
            boundary: crate::runtime_conversations::RuntimeCompactionBoundary {
                retained_group_count: 0,
                compacted_group_count: groups.len(),
            },
            selected_group_start: 0,
            selected_group_end: groups.len().saturating_sub(1),
        };
        let summarizer = LlmBackedCompactionSummarizer::new(|prompt| {
            assert!(prompt.system.contains("agent_compaction"));
            assert!(prompt.user.contains("keep artifact"));
            Ok(RuntimeIterativeSummary::default())
        });

        let summary = summarizer
            .summarize(CompactionSummaryInput {
                prior: None,
                compacted_groups: &groups,
                rows: &rows,
                decision: &decision,
            })
            .expect("summary");

        assert!(summary
            .active_user_goals
            .iter()
            .any(|item| item.contains("keep artifact")));
        assert!(summary.unresolved_followups.is_empty());
    }
}
