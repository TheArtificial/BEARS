use crate::runtime_conversations::RuntimeSemanticGroup;

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
    let _ = decision;
    let mut summary = merge_iterative_summary(prior, compacted_groups);
    enrich_summary_from_rows(&mut summary, rows, compacted_groups);
    summary
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
    use crate::runtime_conversations::{RuntimeCompactionTriggerKind, RuntimeSemanticGroup, RuntimeSemanticGroupKind};
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
}
