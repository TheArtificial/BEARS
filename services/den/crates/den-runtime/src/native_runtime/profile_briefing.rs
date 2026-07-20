use std::fmt::Write as _;

use crate::memory_curate_executor::CurateBriefingItem;

pub fn compose_curate_briefing_prompt(briefing: &[CurateBriefingItem]) -> String {
    let mut prompt = String::from(
        "You are the Curate profile reviewing deferred memory proposals. \
Summarize the briefing below for the operator: what needs attention, why, and recommended next steps. \
Keep the response concise and actionable.\n\n",
    );
    for (index, item) in briefing.iter().enumerate() {
        let _ = writeln!(
            prompt,
            "## Proposal {} — {}\n\
- proposal_id: {}\n\
- status: {} (triage: {})\n\
- source_profile: {}\n\
- suggested_action: {}\n\
- summary: {}\n",
            index + 1,
            item.title,
            item.proposal_id,
            item.status,
            item.triage,
            item.source_profile,
            item.suggested_action,
            item.summary,
        );
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn compose_curate_briefing_prompt_includes_proposal_fields() {
        let proposal_id = Uuid::new_v4();
        let prompt = compose_curate_briefing_prompt(&[CurateBriefingItem {
            proposal_id,
            title: "Promote onboarding note".to_string(),
            summary: "Pair captured a durable preference.".to_string(),
            suggested_action: "promote_to_core".to_string(),
            source_profile: "pair".to_string(),
            status: "deferred".to_string(),
            triage: "escalate_human".to_string(),
        }]);
        assert!(prompt.contains("Curate profile reviewing deferred memory proposals"));
        assert!(prompt.contains("Promote onboarding note"));
        assert!(prompt.contains(&proposal_id.to_string()));
        assert!(prompt.contains("promote_to_core"));
        assert!(prompt.contains("escalate_human"));
    }

    #[test]
    fn compose_curate_briefing_prompt_numbers_multiple_items() {
        let prompt = compose_curate_briefing_prompt(&[
            CurateBriefingItem {
                proposal_id: Uuid::new_v4(),
                title: "First".to_string(),
                summary: "One".to_string(),
                suggested_action: "defer".to_string(),
                source_profile: "pair".to_string(),
                status: "deferred".to_string(),
                triage: "defer".to_string(),
            },
            CurateBriefingItem {
                proposal_id: Uuid::new_v4(),
                title: "Second".to_string(),
                summary: "Two".to_string(),
                suggested_action: "reject".to_string(),
                source_profile: "watch".to_string(),
                status: "needs_human_review".to_string(),
                triage: "escalate_human".to_string(),
            },
        ]);
        assert!(prompt.contains("## Proposal 1 — First"));
        assert!(prompt.contains("## Proposal 2 — Second"));
    }
}
