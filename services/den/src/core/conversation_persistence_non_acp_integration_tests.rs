use sqlx::PgPool;
use uuid::Uuid;

async fn persist_for_test(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    scope_id: &str,
    projection: Projection,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = canonical_persistence_context(
        pool.clone(),
        bear_id,
        None,
        conversation_id.to_string(),
        None,
        None,
        scope_id.to_string(),
        false,
    );
    persist_projection(&context, &projection).await?;
    Ok(())
}

use crate::core::{
    bears::{db::BearParams, db::create_bear, BearAgentRole},
    conversation_events::{
        canonical_persistence_context, persist_projection, MemoryCurateEnqueuedPayload,
        MemoryProposalCreatedPayload, MemoryProposalResolvedPayload,
        PairReflectionCompletedPayload, Projection, ProjectionEvent, ProjectionProvenance,
        ProjectionSource,
    },
    conversation_persistence::{ensure_conversation_for_external_id, list_messages_page},
    memory_proposals::{self, CreateMemoryProposal, ProposalResolutionParams},
    reflection_conductor::{self, ProposalEnqueueParams},
};

#[sqlx::test]
async fn non_acp_memory_proposal_projection_persists_workflow_and_visible_messages(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bear_id = create_bear(
        &pool,
        BearParams {
            slug: "test-memory-proposal-bear",
            name: "Test Memory Proposal Bear",
            description: "test",
            system_prompt: "test",
            default_model: None,
            tools_enabled: None,
            letta_agent_type: None,
            letta_tool_ids: sqlx::types::Json(vec![]),
            context_profile: None,
        },
    )
    .await?;
    let conversation = ensure_conversation_for_external_id(
        &pool,
        bear_id,
        None,
        "conv-memory-proposal-test",
        None,
        None,
    )
    .await?;
    let proposal = memory_proposals::create(
        &pool,
        CreateMemoryProposal {
            bear_id,
            source_role: BearAgentRole::Pair,
            source_agent_id: Some("agent-pair".to_string()),
            source_paths: vec!["pair/notes/test.md".to_string()],
            source_refs: serde_json::json!({
                "conversation_id": "conv-memory-proposal-test"
            }),
            suggested_action: "retain_role_local",
            target_ref: None,
            title: "Test proposal",
            summary: "Test summary",
            rationale: "Because testing",
            proposed_content: None,
            proposed_patch: None,
            refs: serde_json::json!({}),
            sensitivity: "normal",
            requires_human: false,
            project_to_conversation: true,
        },
    )
    .await?;

    let resolved = memory_proposals::resolve_for_bear(
        &pool,
        ProposalResolutionParams {
            bear_id,
            proposal_id: proposal.id,
            reviewer_role: BearAgentRole::Curate,
            reviewer_agent_id: Some("agent-curate"),
            status: "approved",
            review_notes: Some("looks good"),
            decision_summary: Some("applied"),
            result_path: Some("core/test.md"),
            result_commit: Some("abc123"),
            project_to_conversation: true,
        },
    )
    .await?;
    persist_for_test(
        &pool,
        bear_id,
        "conv-memory-proposal-test",
        &format!("bear:{bear_id}"),
        Projection {
            provenance: ProjectionProvenance {
                source: ProjectionSource::MemoryProposals,
                scope_id: format!("bear:{bear_id}"),
            },
            event: ProjectionEvent::MemoryProposalCreated(MemoryProposalCreatedPayload {
                proposal_id: proposal.id,
                source_role: proposal.source_role.clone(),
                suggested_action: proposal.suggested_action.clone(),
                title: proposal.title.clone(),
                status: proposal.status.clone(),
            }),
            workflow_text: format!("Memory proposal created: {}", proposal.title),
            visible_summary: Some(format!(
                "Review requested for memory proposal '{}' from {}.",
                proposal.title, proposal.source_role
            )),
        },
    )
    .await?;
    persist_for_test(
        &pool,
        bear_id,
        "conv-memory-proposal-test",
        &format!("bear:{bear_id}"),
        Projection {
            provenance: ProjectionProvenance {
                source: ProjectionSource::MemoryProposals,
                scope_id: format!("bear:{bear_id}"),
            },
            event: ProjectionEvent::MemoryProposalResolved(MemoryProposalResolvedPayload {
                proposal_id: resolved.id,
                source_role: resolved.source_role.clone(),
                suggested_action: resolved.suggested_action.clone(),
                title: resolved.title.clone(),
                status: resolved.status.clone(),
                reviewer_role: resolved.reviewer_role.clone(),
                result_path: resolved.result_path.clone(),
                result_commit: resolved.result_commit.clone(),
            }),
            workflow_text: format!("Memory proposal resolved: {} ({})", resolved.title, resolved.status),
            visible_summary: Some(format!(
                "Memory proposal '{}' was approved and applied at core/test.md.",
                resolved.title
            )),
        },
    )
    .await?;

    let messages = list_messages_page(&pool, conversation.id, None, 20).await?;
    let texts: Vec<_> = messages.iter().map(|m| m.content_text.as_str()).collect();
    assert!(texts.iter().any(|text| text.contains("Memory proposal created")));
    assert!(texts.iter().any(|text| text.contains("Review requested for memory proposal")));
    assert!(texts.iter().any(|text| text.contains("Memory proposal resolved")));
    assert!(texts.iter().any(|text| text.contains("was approved and applied at core/test.md")));
    assert!(messages.iter().any(|m| m.message_type == "workflow_event"));
    assert!(messages.iter().any(|m| m.role.as_deref() == Some("assistant")));
    Ok(())
}

#[sqlx::test]
async fn non_acp_pair_reflection_completion_persists_records_when_conversation_is_valid(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bear_id = create_bear(
        &pool,
        BearParams {
            slug: "test-pair-reflection-bear",
            name: "Test Pair Reflection Bear",
            description: "test",
            system_prompt: "test",
            default_model: None,
            tools_enabled: None,
            letta_agent_type: None,
            letta_tool_ids: sqlx::types::Json(vec![]),
            context_profile: None,
        },
    )
    .await?;
    let conversation = ensure_conversation_for_external_id(
        &pool,
        bear_id,
        None,
        "conv-pair-reflection-test",
        None,
        None,
    )
    .await?;
    let projection = Projection {
        provenance: ProjectionProvenance {
            source: ProjectionSource::PairReflection,
            scope_id: format!("bear:{bear_id}:acp:acp-test-session"),
        },
        event: ProjectionEvent::PairReflectionCompleted(PairReflectionCompletedPayload {
            reflection_run_id: Uuid::new_v4(),
            acp_session_id: "acp-test-session".to_string(),
            trigger: "manual".to_string(),
            status: "completed".to_string(),
            summary_path: Some("pair/summary.md".to_string()),
            summary_commit: Some("def456".to_string()),
            considered_message_count: 3,
            completed_at: None,
        }),
        workflow_text: "Pair reflection completed for session acp-test-session".to_string(),
        visible_summary: Some(
            "Pair reflection summary completed for session acp-test-session and saved to pair/summary.md.".to_string(),
        ),
    };
    persist_for_test(
        &pool,
        bear_id,
        "conv-pair-reflection-test",
        &format!("bear:{bear_id}:acp:acp-test-session"),
        projection,
    )
    .await?;

    let messages = list_messages_page(&pool, conversation.id, None, 20).await?;
    assert!(messages.iter().any(|m| m.content_text.contains("Pair reflection completed for session acp-test-session")));
    assert!(messages.iter().any(|m| m.content_text.contains("Pair reflection summary completed for session acp-test-session and saved to pair/summary.md.")));
    Ok(())
}

#[sqlx::test]
async fn non_acp_memory_curate_enqueue_projection_respects_conversation_gating(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bear_id = create_bear(
        &pool,
        BearParams {
            slug: "test-memory-curate-bear",
            name: "Test Memory Curate Bear",
            description: "test",
            system_prompt: "test",
            default_model: None,
            tools_enabled: None,
            letta_agent_type: None,
            letta_tool_ids: sqlx::types::Json(vec![]),
            context_profile: None,
        },
    )
    .await?;
    let conversation = ensure_conversation_for_external_id(
        &pool,
        bear_id,
        None,
        "conv-memory-curate-test",
        None,
        None,
    )
    .await?;
    let proposal_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
    let _ = reflection_conductor::enqueue_memory_curate_for_proposals(
        &pool,
        ProposalEnqueueParams {
            bear_id,
            role_agent_id: Some("agent-pair"),
            conversation_id: Some("conv-memory-curate-test"),
            conversation_key: Some("conv-key"),
            conversation_date: None,
            trigger: "pair_reflection",
            proposal_ids: proposal_ids.clone(),
        },
    )
    .await?;
    persist_for_test(
        &pool,
        bear_id,
        "conv-memory-curate-test",
        &format!("bear:{bear_id}:lane:memory_curate"),
        Projection {
            provenance: ProjectionProvenance {
                source: ProjectionSource::ReflectionConductor,
                scope_id: format!("bear:{bear_id}:lane:memory_curate"),
            },
            event: ProjectionEvent::MemoryCurateEnqueued(MemoryCurateEnqueuedPayload {
                reflection_run_id: Uuid::new_v4(),
                lane: "memory_curate".to_string(),
                trigger: "pair_reflection".to_string(),
                status: "queued".to_string(),
                proposal_ids,
                conversation_key: Some("conv-key".to_string()),
                conversation_date: None,
                created_at: time::OffsetDateTime::now_utc(),
            }),
            workflow_text: "Memory curate enqueued with 2 proposal(s)".to_string(),
            visible_summary: Some("Memory curate was queued for 2 proposal(s).".to_string()),
        },
    )
    .await?;

    let messages = list_messages_page(&pool, conversation.id, None, 20).await?;
    assert!(messages.iter().any(|m| m.content_text.contains("Memory curate enqueued with 2 proposal(s)")));
    assert!(messages.iter().any(|m| m.content_text.contains("Memory curate was queued for 2 proposal(s).")));
    assert!(!messages.iter().any(|m| m.content_text.contains("Should not persist")));
    Ok(())
}
