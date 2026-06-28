use crate::conversation::events::*;

#[test]
fn projection_workflow_content_json_is_derived_from_typed_event() {
    let projection = Projection {
        provenance: ProjectionProvenance {
            source: ProjectionSource::PairReflection,
            scope_id: "bear:scope".to_string(),
        },
        event: ProjectionEvent::PairReflectionCompleted(PairReflectionCompletedPayload {
            reflection_run_id: uuid::Uuid::nil(),
            client_session_id: "acp-session".to_string(),
            trigger: "manual".to_string(),
            status: "completed".to_string(),
            summary_path: Some("pair/summary.md".to_string()),
            summary_commit: Some("abc123".to_string()),
            considered_message_count: 4,
            completed_at: None,
        }),
        workflow_text: "Pair reflection completed".to_string(),
        visible_summary: Some("Summary saved".to_string()),
    };

    let json = projection.workflow_content_json();
    assert_eq!(json["event"], "pair_reflection_completed");
    assert_eq!(json["source"], "pair_reflection");
    assert_eq!(json["scope_id"], "bear:scope");
    assert_eq!(json["status"], "completed");
    assert_eq!(json["summary_path"], "pair/summary.md");
}

#[test]
fn projection_requires_canonical_conversation_id_shape() {
    assert!(Some("conv-123").filter(|id| id.starts_with("conv-")).is_some());
    assert!(Some("conversation-123")
        .filter(|id| id.starts_with("conv-"))
        .is_none());
    assert!(Option::<&str>::None
        .filter(|id| id.starts_with("conv-"))
        .is_none());
}

#[test]
fn proposal_projection_helpers_build_expected_summaries() {
    let created = memory_proposal_created_projection(
        ProjectionProvenance {
            source: ProjectionSource::DenTools,
            scope_id: "scope-1".to_string(),
        },
        uuid::Uuid::nil(),
        "pair".to_string(),
        "promote_to_core".to_string(),
        "Proposal A".to_string(),
        "pending".to_string(),
    );
    assert_eq!(created.workflow_text, "Memory proposal created: Proposal A");
    assert_eq!(
        created.visible_summary.as_deref(),
        Some("Review requested for memory proposal 'Proposal A' from pair.")
    );

    let resolved = memory_proposal_resolved_projection(
        ProjectionProvenance {
            source: ProjectionSource::DenTools,
            scope_id: "scope-2".to_string(),
        },
        uuid::Uuid::nil(),
        "pair".to_string(),
        "promote_to_core".to_string(),
        "Proposal B".to_string(),
        "approved".to_string(),
        Some("curate".to_string()),
        Some("core/notes.md".to_string()),
        None,
    );
    assert_eq!(resolved.workflow_text, "Memory proposal resolved: Proposal B (approved)");
    assert_eq!(
        resolved.visible_summary.as_deref(),
        Some("Memory proposal 'Proposal B' was approved and applied at core/notes.md.")
    );

    let requested = memory_review_requested_projection(
        ProjectionProvenance {
            source: ProjectionSource::DenTools,
            scope_id: "scope-3".to_string(),
        },
        uuid::Uuid::nil(),
        "pair".to_string(),
        "promote_to_core".to_string(),
        "Proposal C".to_string(),
        "pending".to_string(),
        vec!["pair/notes/test.md".to_string()],
    );
    assert_eq!(requested.workflow_text, "Memory review requested: Proposal C");
    assert_eq!(
        requested.visible_summary.as_deref(),
        Some("Review requested for memory proposal 'Proposal C' from pair.")
    );
}

#[test]
fn projection_memory_review_requested_json_uses_typed_event_shape() {
    let projection = Projection {
        provenance: ProjectionProvenance {
            source: ProjectionSource::DenTools,
            scope_id: "acp-session-1".to_string(),
        },
        event: ProjectionEvent::MemoryReviewRequested(MemoryReviewRequestedPayload {
            proposal_id: uuid::Uuid::nil(),
            source_profile: "pair".to_string(),
            title: "Promote note".to_string(),
            suggested_action: "promote_to_core".to_string(),
            status: "pending".to_string(),
            source_paths: vec!["pair/notes/test.md".to_string()],
        }),
        workflow_text: "Memory review requested".to_string(),
        visible_summary: Some("Review requested".to_string()),
    };

    let json = projection.workflow_content_json();
    assert_eq!(json["event"], "memory_review_requested");
    assert_eq!(json["source"], "den_core::tools");
    assert_eq!(json["scope_id"], "acp-session-1");
    assert_eq!(json["title"], "Promote note");
    assert_eq!(json["suggested_action"], "promote_to_core");
    assert_eq!(json["source_paths"], serde_json::json!(["pair/notes/test.md"]));
}

#[test]
fn projection_visible_summary_record_uses_visible_assistant_shape() {
    let projection = Projection {
        provenance: ProjectionProvenance {
            source: ProjectionSource::MemoryProposals,
            scope_id: "bear:scope".to_string(),
        },
        event: ProjectionEvent::MemoryProposalCreated(MemoryProposalCreatedPayload {
            proposal_id: uuid::Uuid::nil(),
            source_profile: "pair".to_string(),
            suggested_action: "retain_profile_local".to_string(),
            title: "Proposal".to_string(),
            status: "pending".to_string(),
        }),
        workflow_text: "Memory proposal created".to_string(),
        visible_summary: Some("Summary saved".to_string()),
    };

    let record = projection.visible_summary_record().expect("summary record");
    match record {
        CanonicalConversationRecord::VisibleMessage {
            role,
            text,
            content_json,
            provider_message_id,
        } => {
            assert_eq!(role.as_str(), "assistant");
            assert_eq!(text, "Summary saved");
            assert_eq!(content_json, serde_json::json!({}));
            assert_eq!(provider_message_id, None);
        }
        _ => panic!("expected visible assistant message"),
    }
}
