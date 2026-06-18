use crate::runtime_conversations::RuntimeIterativeSummary;

pub fn render_compacted_context_block(summary: &RuntimeIterativeSummary) -> String {
    let mut sections = Vec::new();
    sections.push("# Compacted context".to_string());

    if !summary.active_user_goals.is_empty() {
        sections.push(format!(
            "## Active goals\n{}",
            bullet_list(&summary.active_user_goals)
        ));
    }
    if !summary.important_constraints.is_empty() {
        sections.push(format!(
            "## Constraints\n{}",
            bullet_list(&summary.important_constraints)
        ));
    }
    if !summary.decisions_made.is_empty() {
        sections.push(format!(
            "## Decisions\n{}",
            bullet_list(&summary.decisions_made)
        ));
    }
    if !summary.artifact_refs.is_empty() {
        sections.push(format!(
            "## Artifacts\n{}",
            bullet_list(&summary.artifact_refs)
        ));
    }
    if !summary.workflow_state_refs.is_empty() {
        sections.push(format!(
            "## Workflow state\n{}",
            bullet_list(&summary.workflow_state_refs)
        ));
    }
    if !summary.unresolved_followups.is_empty() {
        sections.push(format!(
            "## Unresolved follow-ups\n{}",
            bullet_list(&summary.unresolved_followups)
        ));
    }

    if sections.len() == 1 {
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
