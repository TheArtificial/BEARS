use serde_json::json;

use crate::{
    runtime::compaction::{
        artifact_ref_from_decision, build_runtime_context_envelope, choose_compaction_decision,
        merge_iterative_summary, semantic_groups_from_conversation_messages,
        semantic_groups_from_runtime_messages, RuntimeCompactionDecision, RuntimeCompactionPolicy,
        RuntimeCompactionStrategy, RuntimeContextEnvelopeInput, TranscriptGroupingRow,
    },
    runtime_conversations::{
        RuntimeCompactionArtifactKind, RuntimeCompactionBoundary, RuntimeCompactionTriggerKind,
        RuntimeIterativeSummary, RuntimeSemanticGroup, RuntimeSemanticGroupKind,
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
    assert_eq!(
        groups[3].kind,
        RuntimeSemanticGroupKind::ApprovalInteraction
    );
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
        max_transcript_chars: 20_000,
    };

    let decision = choose_compaction_decision(
        &groups,
        RuntimeCompactionTriggerKind::SemanticGroupCount,
        &policy,
    )
    .expect("decision");

    assert_eq!(decision.selected_group_start, 0);
    assert_eq!(decision.selected_group_end, 1);
    assert_eq!(
        decision.strategy,
        RuntimeCompactionStrategy::UpdateIterativeSummary
    );
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
        max_transcript_chars: 20_000,
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
        max_transcript_chars: 20_000,
    };

    let artifact = artifact_ref_from_decision("artifact-1", &decision, &policy);
    assert_eq!(
        artifact.kind,
        RuntimeCompactionArtifactKind::CollapsedToolBundle
    );
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
    assert!(merged
        .workflow_state_refs
        .iter()
        .any(|v| v.contains("WorkflowUpdate:w1:w2")));
    assert!(merged
        .artifact_refs
        .iter()
        .any(|v| v.contains("ArtifactUpdate:a1:a1")));
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

#[test]
fn transcript_grouping_bundles_tool_call_and_result_rows() {
    let groups = semantic_groups_from_conversation_messages(&[
        TranscriptGroupingRow::new("user", "inspect the repo", json!({}))
            .with_message_id("msg-1")
            .with_sequence_no(1),
        TranscriptGroupingRow::new("assistant", "I'll search first", json!({}))
            .with_message_id("msg-2")
            .with_sequence_no(2),
        TranscriptGroupingRow::new(
            "tool_call",
            "Tool request: memory_search",
            json!({
                "event": "tool_request",
                "tool_call_id": "call-1",
                "tool_name": "memory_search",
                "args": {"query": "compaction"},
            }),
        )
        .with_message_id("msg-3")
        .with_sequence_no(3)
        .with_tool_call_id("call-1"),
        TranscriptGroupingRow::new(
            "tool_result",
            "Tool result: memory_search",
            json!({
                "event": "tool_result",
                "tool_call_id": "call-1",
                "tool_name": "memory_search",
                "status": "ok",
                "content": "found notes",
            }),
        )
        .with_message_id("msg-4")
        .with_sequence_no(4)
        .with_tool_call_id("call-1"),
        TranscriptGroupingRow::new("assistant", "here are the notes", json!({}))
            .with_message_id("msg-5")
            .with_sequence_no(5),
    ]);

    assert_eq!(groups.len(), 4);
    assert_eq!(groups[0].kind, RuntimeSemanticGroupKind::UserTurn);
    assert_eq!(groups[1].kind, RuntimeSemanticGroupKind::AssistantReply);
    assert_eq!(groups[2].kind, RuntimeSemanticGroupKind::ToolInteraction);
    assert_eq!(groups[2].message_count, 2);
    assert_eq!(groups[2].start_message_id.as_deref(), Some("msg-3"));
    assert_eq!(groups[2].end_message_id.as_deref(), Some("msg-4"));
    assert!(!groups[2].protected);
    assert_eq!(groups[3].kind, RuntimeSemanticGroupKind::AssistantReply);
}

#[test]
fn transcript_grouping_marks_orphan_tool_call_as_protected() {
    let groups = semantic_groups_from_conversation_messages(&[
        TranscriptGroupingRow::new(
            "tool_call",
            "Tool request: fs_edit_file",
            json!({
                "event": "tool_request",
                "tool_call_id": "call-orphan",
                "tool_name": "fs_edit_file",
                "args": {"path": "README.md"},
            }),
        )
        .with_message_id("msg-orphan")
        .with_sequence_no(10)
        .with_tool_call_id("call-orphan"),
        TranscriptGroupingRow::new("assistant", "continuing without result", json!({}))
            .with_message_id("msg-after")
            .with_sequence_no(11),
    ]);

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].kind, RuntimeSemanticGroupKind::ToolInteraction);
    assert_eq!(groups[0].message_count, 1);
    assert!(groups[0].protected);
    assert_eq!(groups[0].start_message_id.as_deref(), Some("msg-orphan"));
}

#[test]
fn transcript_grouping_classifies_pending_approval_tool_span() {
    let groups = semantic_groups_from_conversation_messages(&[
        TranscriptGroupingRow::new(
            "tool_call",
            "Tool request: fs_edit_file",
            json!({
                "event": "tool_request",
                "tool_call_id": "call-approval",
                "tool_name": "fs_edit_file",
                "approval_request_id": "approval-1",
                "approval_required": true,
                "args": {"path": "README.md"},
            }),
        )
        .with_message_id("msg-call")
        .with_sequence_no(20)
        .with_tool_call_id("call-approval"),
        TranscriptGroupingRow::new(
            "workflow_event",
            "Approval granted",
            json!({
                "event": "approval_decision",
                "approval_request_id": "approval-1",
                "decision": "approve",
            }),
        )
        .with_message_id("msg-approval")
        .with_sequence_no(21),
    ]);

    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[0].kind,
        RuntimeSemanticGroupKind::ApprovalInteraction
    );
    assert!(groups[0].protected);
    assert_eq!(
        groups[1].kind,
        RuntimeSemanticGroupKind::ApprovalInteraction
    );
    assert!(groups[1].protected);
}

#[test]
fn transcript_grouping_resolves_approval_tool_pair_after_result() {
    let groups = semantic_groups_from_conversation_messages(&[
        TranscriptGroupingRow::new(
            "tool_call",
            "Tool request: fs_edit_file",
            json!({
                "event": "tool_request",
                "tool_call_id": "call-resolved",
                "tool_name": "fs_edit_file",
                "approval_request_id": "approval-2",
                "approval_required": true,
                "args": {"path": "README.md"},
            }),
        )
        .with_message_id("msg-call")
        .with_sequence_no(30)
        .with_tool_call_id("call-resolved"),
        TranscriptGroupingRow::new(
            "tool_result",
            "Tool result: fs_edit_file",
            json!({
                "event": "tool_result",
                "tool_call_id": "call-resolved",
                "tool_name": "fs_edit_file",
                "approval_request_id": "approval-2",
                "status": "ok",
                "content": "edited",
            }),
        )
        .with_message_id("msg-result")
        .with_sequence_no(31)
        .with_tool_call_id("call-resolved"),
    ]);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].kind, RuntimeSemanticGroupKind::ToolInteraction);
    assert_eq!(groups[0].message_count, 2);
    assert!(!groups[0].protected);
}

#[test]
fn transcript_grouping_treats_compaction_marker_as_protected_boundary() {
    let groups = semantic_groups_from_conversation_messages(&[
        TranscriptGroupingRow::new("user", "older question", json!({}))
            .with_message_id("msg-old")
            .with_sequence_no(1),
        TranscriptGroupingRow::new(
            "compaction_marker",
            "Compaction boundary",
            json!({
                "event": "compaction_applied",
                "artifact_id": "artifact-1",
            }),
        )
        .with_message_id("msg-marker")
        .with_sequence_no(2),
        TranscriptGroupingRow::new("assistant", "continuing after compaction", json!({}))
            .with_message_id("msg-new")
            .with_sequence_no(3),
    ]);

    assert_eq!(groups.len(), 3);
    assert_eq!(
        groups[1].kind,
        RuntimeSemanticGroupKind::PriorCompactionArtifact
    );
    assert!(groups[1].protected);
    assert_eq!(groups[1].start_message_id.as_deref(), Some("msg-marker"));
}

#[test]
fn transcript_grouping_handles_multiple_consecutive_tool_bundles() {
    let groups = semantic_groups_from_conversation_messages(&[
        TranscriptGroupingRow::new(
            "tool_call",
            "Tool request: memory_browse",
            json!({
                "event": "tool_request",
                "tool_call_id": "call-a",
                "tool_name": "memory_browse",
                "args": {},
            }),
        )
        .with_message_id("msg-a-call")
        .with_sequence_no(40),
        TranscriptGroupingRow::new(
            "tool_result",
            "Tool result: memory_browse",
            json!({
                "event": "tool_result",
                "tool_call_id": "call-a",
                "tool_name": "memory_browse",
                "status": "ok",
                "content": "tree",
            }),
        )
        .with_message_id("msg-a-result")
        .with_sequence_no(41),
        TranscriptGroupingRow::new(
            "tool_call",
            "Tool request: memory_read",
            json!({
                "event": "tool_request",
                "tool_call_id": "call-b",
                "tool_name": "memory_read",
                "args": {"path": "core/notes.md"},
            }),
        )
        .with_message_id("msg-b-call")
        .with_sequence_no(42),
        TranscriptGroupingRow::new(
            "tool_result",
            "Tool result: memory_read",
            json!({
                "event": "tool_result",
                "tool_call_id": "call-b",
                "tool_name": "memory_read",
                "status": "incomplete",
                "content": null,
            }),
        )
        .with_message_id("msg-b-result")
        .with_sequence_no(43),
    ]);

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].message_count, 2);
    assert!(!groups[0].protected);
    assert_eq!(groups[1].message_count, 2);
    assert!(groups[1].protected);
}
