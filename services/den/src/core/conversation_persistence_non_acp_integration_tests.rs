#![cfg(test)]

use sqlx::PgPool;
use uuid::Uuid;

async fn persist_projection_pair(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    scope_id: &str,
    workflow_text: &str,
    workflow_json: serde_json::Value,
    summary_text: Option<&str>,
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
    persist_canonical_conversation_record(
        &context,
        &CanonicalConversationRecord::workflow_event(
            workflow_text.to_string(),
            workflow_json,
            None,
        ),
    )
    .await?;
    if let Some(summary_text) = summary_text {
        persist_canonical_conversation_record(
            &context,
            &CanonicalConversationRecord::visible_assistant_message(
                summary_text.to_string(),
                serde_json::json!({}),
                None,
            ),
        )
        .await?;
    }
    Ok(())
}

use crate::core::{
    bears::{db::BearParams, db::create_bear, BearAgentRole},
    conversation_events::{canonical_persistence_context, persist_canonical_conversation_record, CanonicalConversationRecord},
    conversation_events::{
        project_non_acp_audit_event, ConversationEventProvenance, NonAcpAuditProjection,
    },
    conversation_persistence::{ensure_conversation_for_external_id, list_messages_page},
    memory_proposals::{self, CreateMemoryProposal, ProposalResolutionParams},
    pair_reflection::{self, CompletePairReflectionRun, CreatePairReflectionRun},
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
        },
    )
    .await?;

    let _ = memory_proposals::resolve_for_bear(
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
        },
    )
    .await?;

    let messages = list_messages_page(&pool, conversation.id, None, 20).await?;
    let texts: Vec<_> = messages.iter().map(|m| m.content_text.as_str()).collect();
    println!("memory proposal texts: {:?}", texts);
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
    let run = pair_reflection::create_run(
        &pool,
        CreatePairReflectionRun {
            bear_id,
            user_id: 7,
            acp_session_id: "acp-test-session",
            conversation_id: Some("conv-pair-reflection-test"),
            trigger: "manual",
            considered_message_count: 3,
            considered_memory_paths: vec!["pair/summary.md".to_string()],
            diagnostic: serde_json::json!({}),
        },
    )
    .await?;
    let _ = pair_reflection::complete_run(
        &pool,
        CompletePairReflectionRun {
            id: run.id,
            status: "completed",
            summary_path: Some("pair/summary.md"),
            summary_commit: Some("def456"),
            diagnostic: serde_json::json!({}),
        },
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
    let _ = reflection_conductor::enqueue_memory_curate_for_proposals(
        &pool,
        ProposalEnqueueParams {
            bear_id,
            role_agent_id: Some("agent-pair"),
            conversation_id: Some("conv-memory-curate-test"),
            conversation_key: Some("conv-key"),
            conversation_date: None,
            trigger: "pair_reflection",
            proposal_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
        },
    )
    .await?;

    project_non_acp_audit_event(
        &pool,
        bear_id,
        None,
        Some("not-a-conv-id"),
        ConversationEventProvenance {
            source: "test".to_string(),
            scope_id: "bear:test".to_string(),
        },
        NonAcpAuditProjection {
            event: "ignored".to_string(),
            workflow_text: "Should not persist".to_string(),
            workflow_json: serde_json::json!({}),
            visible_summary_text: Some("Should not persist".to_string()),
        },
    );
    tokio::task::yield_now().await;

    let messages = list_messages_page(&pool, conversation.id, None, 20).await?;
    println!("memory curate texts: {:?}", messages.iter().map(|m| m.content_text.as_str()).collect::<Vec<_>>());
    assert!(messages.iter().any(|m| m.content_text.contains("Memory curate enqueued with 2 proposal(s)")));
    assert!(messages.iter().any(|m| m.content_text.contains("Memory curate was queued for 2 proposal(s).")));
    assert!(!messages.iter().any(|m| m.content_text.contains("Should not persist")));
    Ok(())
}
