use crate::runtime_conversations::RuntimeIterativeSummary;

pub fn render_compacted_context_block(summary: &RuntimeIterativeSummary) -> String {
    let mut sections = Vec::new();
    sections.push("# Compacted context".to_string());

    let mut rendered_summary_fields = false;
    for (heading, items) in [
        ("Active goals", summary.active_user_goals.as_slice()),
        ("Constraints", summary.important_constraints.as_slice()),
        ("Decisions", summary.decisions_made.as_slice()),
        ("Artifacts", summary.artifact_refs.as_slice()),
        ("Workflow state", summary.workflow_state_refs.as_slice()),
        (
            "Unresolved follow-ups",
            summary.unresolved_followups.as_slice(),
        ),
    ] {
        if items.is_empty() {
            continue;
        }
        rendered_summary_fields = true;
        sections.push(format!("## {heading}\n{}", bullet_list(items)));
    }

    if !rendered_summary_fields {
        sections.push(
            "_No compacted continuity signals yet; older transcript groups were folded without structured summary fields._"
                .to_string(),
        );
    }

    sections.join("\n\n")
}

fn bullet_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_compacted_context_block_formats_sections() {
        let summary = RuntimeIterativeSummary {
            active_user_goals: vec!["ship compaction".into()],
            important_constraints: vec!["do not compact approvals".into()],
            decisions_made: vec![],
            artifact_refs: vec![],
            workflow_state_refs: vec![],
            unresolved_followups: vec![],
        };
        let rendered = render_compacted_context_block(&summary);
        assert!(rendered.contains("# Compacted context"));
        assert!(rendered.contains("ship compaction"));
        assert!(rendered.contains("do not compact approvals"));
    }
}
