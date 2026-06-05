use serde_json::json;

use crate::core::{
    runtime::compaction::{
        artifact_ref_from_decision, build_runtime_context_envelope,
        choose_compaction_decision, merge_iterative_summary,
        semantic_groups_from_runtime_messages, RuntimeCompactionDecision,
        RuntimeCompactionPolicy, RuntimeCompactionStrategy, RuntimeContextEnvelopeInput,
    },
    runtime_conversations::{
        RuntimeCompactionArtifactKind, RuntimeCompactionBoundary,
        RuntimeCompactionTriggerKind, RuntimeIterativeSummary, RuntimeSemanticGroup,
        RuntimeSemanticGroupKind,
    },
};

#[test]
fn semantic_grouping_classifies_user_assistant_tool_and_approval_messages() {
    let groups = semantic_groups_from_runtime_messages(&[
        json!({"role": "user", "content": "please inspect this file"}),
        json!({"role": "assistant", "content": "I will inspect it"}),
        json!({"role": "tool", "tool_call_id": "call-1", "tool_name": "fs_read_text_file"}),
        json!({"role": "assistant", "approval_request_id": "approval-1", "content": "approval required"}),
        json!({"role": "assistant", "content": "workflow_state: submitted"}),
        json!({"role": "assistant", "content": "artifact saved to file:///tmp/output.md"}),
    ]);

    assert_eq!(groups[0].kind, RuntimeSemanticGroupKind::UserTurn);
    assert_eq!(groups[1].kind, RuntimeSemanticGroupKind::AssistantReply);
    assert_eq!(groups[2].kind, RuntimeSemanticGroupKind::ToolInteraction);
    assert!(groups[2].protected);
    assert_eq!(groups[3].kind, RuntimeSemanticGroupKind::ApprovalInteraction);
    assert!(groups[3].protected);
    assert_eq!(groups[4].kind, RuntimeSemanticGroupKind::WorkflowUpdate);
    assert_eq!(groups[5].kind, RuntimeSemanticGroupKind::ArtifactUpdate);
}

#[test]
fn compaction_policy_skips_protected_groups_and_recent_tail() {
    let groups = vec![
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::UserTurn,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: false,
        },
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::AssistantReply,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: false,
        },
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::ApprovalInteraction,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: true,
        },
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::AssistantReply,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: false,
        },
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::UserTurn,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: false,
        },
    ];
    let policy = RuntimeCompactionPolicy {
        policy_version: "v1".into(),
        protected_recent_group_count: 2,
        max_groups_before_compaction: 3,
    };

    let decision = choose_compaction_decision(
        &groups,
        RuntimeCompactionTriggerKind::SemanticGroupCount,
        &policy,
    )
    .expect("decision");

    assert_eq!(decision.selected_group_start, 0);
    assert_eq!(decision.selected_group_end, 1);
    assert_eq!(decision.strategy, RuntimeCompactionStrategy::UpdateIterativeSummary);
    assert_eq!(decision.boundary.compacted_group_count, 2);
}

#[test]
fn compaction_decision_returns_none_when_only_protected_or_recent_groups_remain() {
    let groups = vec![
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::ToolInteraction,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: true,
        },
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::ApprovalInteraction,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: true,
        },
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::UserTurn,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: false,
        },
    ];
    let policy = RuntimeCompactionPolicy {
        policy_version: "v1".into(),
        protected_recent_group_count: 1,
        max_groups_before_compaction: 2,
    };

    assert!(choose_compaction_decision(
        &groups,
        RuntimeCompactionTriggerKind::TokenPressure,
        &policy,
    )
    .is_none());
}

#[test]
fn artifact_ref_carries_policy_version_and_source_range() {
    let decision = RuntimeCompactionDecision {
        trigger: RuntimeCompactionTriggerKind::Manual,
        strategy: RuntimeCompactionStrategy::CollapseToolBundles,
        boundary: RuntimeCompactionBoundary {
            retained_group_count: 4,
            compacted_group_count: 2,
        },
        selected_group_start: 1,
        selected_group_end: 2,
    };
    let policy = RuntimeCompactionPolicy {
        policy_version: "policy-7".into(),
        protected_recent_group_count: 2,
        max_groups_before_compaction: 5,
    };

    let artifact = artifact_ref_from_decision("artifact-1", &decision, &policy);
    assert_eq!(artifact.kind, RuntimeCompactionArtifactKind::CollapsedToolBundle);
    assert_eq!(artifact.source_group_start, 1);
    assert_eq!(artifact.source_group_end, 2);
    assert_eq!(artifact.policy_version, "policy-7");
}

#[test]
fn iterative_summary_merge_accumulates_unique_entries_and_protected_markers() {
    let prior = RuntimeIterativeSummary {
        active_user_goals: vec!["UserTurn:start:end".into()],
        important_constraints: vec![],
        decisions_made: vec![],
        artifact_refs: vec![],
        workflow_state_refs: vec![],
        unresolved_followups: vec![],
    };
    let groups = vec![
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::UserTurn,
            start_message_id: None,
            end_message_id: None,
            message_count: 1,
            protected: false,
        },
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::WorkflowUpdate,
            start_message_id: Some("w1".into()),
            end_message_id: Some("w2".into()),
            message_count: 2,
            protected: true,
        },
        RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::ArtifactUpdate,
            start_message_id: Some("a1".into()),
            end_message_id: Some("a1".into()),
            message_count: 1,
            protected: false,
        },
    ];

    let merged = merge_iterative_summary(Some(&prior), &groups);
    assert_eq!(merged.active_user_goals.len(), 1);
    assert!(merged.workflow_state_refs.iter().any(|v| v.contains("WorkflowUpdate:w1:w2")));
    assert!(merged.artifact_refs.iter().any(|v| v.contains("ArtifactUpdate:a1:a1")));
    assert!(merged
        .important_constraints
        .iter()
        .any(|v| v == "protected:WorkflowUpdate"));
}

#[test]
fn prompt_assembly_keeps_compacted_context_separate_from_recent_groups() {
    let envelope = build_runtime_context_envelope(RuntimeContextEnvelopeInput {
        active_instructions: vec!["system".into(), "developer".into()],
        workflow_state: vec!["plan:active".into()],
        recent_groups: vec![RuntimeSemanticGroup {
            kind: RuntimeSemanticGroupKind::AssistantReply,
            start_message_id: Some("m1".into()),
            end_message_id: Some("m1".into()),
            message_count: 1,
            protected: false,
        }],
        compacted_summary: Some(RuntimeIterativeSummary {
            active_user_goals: vec!["ship compaction".into()],
            important_constraints: vec!["do not compact approvals".into()],
            decisions_made: vec![],
            artifact_refs: vec![],
            workflow_state_refs: vec!["plan:active".into()],
            unresolved_followups: vec![],
        }),
    });

    assert_eq!(envelope.instructions, vec!["system", "developer"]);
    assert_eq!(envelope.workflow_state, vec!["plan:active"]);
    assert_eq!(envelope.recent_groups.len(), 1);
    assert!(envelope.compacted_context.is_some());
    assert_eq!(
        envelope.compacted_context.unwrap().important_constraints,
        vec!["do not compact approvals"]
    );
}
