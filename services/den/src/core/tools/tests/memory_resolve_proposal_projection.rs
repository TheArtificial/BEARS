use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::core::{
    bears::{db, db::grant_membership, db::BearParams, BearProfile},
    den_tools::{
        invoke_den_tool, DenToolChannelContext, DenToolInvocationContext,
        DEN_MEMORY_RESOLVE_PROPOSAL,
    },
    memory_proposals::{create, CreateMemoryProposal},
    user::db::create_user,
};

async fn seed_curate_agent(
    pool: &PgPool,
    bear_id: Uuid,
    agent_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
        r#"
        INSERT INTO bear_profile_bindings (bear_id, profile, binding_id, letta_agent_id)
        VALUES ($1, 'curate', $2, $2)
        ON CONFLICT (bear_id, profile)
        DO UPDATE SET letta_agent_id = EXCLUDED.letta_agent_id
        "#,
    )
    .bind(bear_id)
    .bind(agent_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test]
async fn memory_resolve_proposal_projects_typed_conversation_records(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bear_id = db::create_bear(
        &pool,
        BearParams {
            slug: "test-memory-resolve-tool-bear",
            name: "Test Memory Resolve Tool Bear",
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

    let suffix = Uuid::new_v4().simple().to_string();
    let user_id = create_user(
        &pool,
        &format!("rs-{}@ex.com", &suffix[..8]),
        &format!("rs{}", &suffix[..12]),
        "Memory Resolve Tester",
        "test-hash",
    )
    .await?;
    grant_membership(&pool, user_id, bear_id, Some("admin")).await?;

    let agent_id = format!("agent-{}", Uuid::new_v4());
    seed_curate_agent(&pool, bear_id, &agent_id).await?;

    let conversation = crate::core::conversation_persistence::ensure_conversation_for_external_id(
        &pool,
        bear_id,
        Some(user_id),
        "conv-memory-resolve-tool-test",
        None,
        None,
    )
    .await?;

    let proposal = create(
        &pool,
        CreateMemoryProposal {
            bear_id,
            source_role: BearProfile::Pair,
            source_agent_id: Some("agent-pair".to_string()),
            source_paths: vec!["pair/notes/test.md".to_string()],
            source_refs: json!({
                "conversation_id": "conv-memory-resolve-tool-test",
                "session_id": "acp-memory-resolve-tool-session"
            }),
            suggested_action: "promote_to_core",
            target_ref: None,
            title: "Resolve me",
            summary: "candidate",
            rationale: "because",
            proposed_content: None,
            proposed_patch: None,
            refs: json!({}),
            sensitivity: "normal",
            requires_human: false,
            project_to_conversation: false,
        },
    )
    .await?;

    let context = DenToolInvocationContext {
        bear_id,
        bear_slug: "test-memory-resolve-tool-bear".to_string(),
        binding_id: agent_id,
        profile: Some(BearProfile::Curate),
        user_id,
        username: Some("tester".to_string()),
        membership_role: Some("admin".to_string()),
        conversation_id: "conv-memory-resolve-tool-test".to_string(),
        session_id: "acp-memory-resolve-tool-session".to_string(),
        acp_session_id: Some("acp-memory-resolve-tool-session".to_string()),
        conversation_selection: Some("conv-memory-resolve-tool-test".to_string()),
        runtime_target: None,
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        request_id: Some(Uuid::new_v4().to_string()),
        channel: DenToolChannelContext::default(),
    };

    let payload = invoke_den_tool(
        &pool,
        &crate::config::Config::test_stub(),
        DEN_MEMORY_RESOLVE_PROPOSAL,
        json!({
            "proposal_id": proposal.id,
            "status": "rejected",
            "decision_summary": "Not suitable"
        }),
        context,
    )
    .await?;

    assert_eq!(payload["proposal"]["status"], "rejected");

    let projection_context = crate::core::conversation_events::canonical_persistence_context(
        pool.clone(),
        bear_id,
        Some(user_id),
        "conv-memory-resolve-tool-test".to_string(),
        None,
        None,
        "acp-memory-resolve-tool-session".to_string(),
        false,
    );
    crate::core::conversation_events::persist_projection(
        &projection_context,
        &crate::core::conversation_events::memory_proposal_resolved_projection(
            crate::core::conversation_events::ProjectionProvenance {
                source: crate::core::conversation_events::ProjectionSource::DenTools,
                scope_id: "acp-memory-resolve-tool-session".to_string(),
            },
            proposal.id,
            "pair".to_string(),
            "promote_to_core".to_string(),
            "Resolve me".to_string(),
            "rejected".to_string(),
            Some("curate".to_string()),
            None,
            None,
        ),
    )
    .await?;

    let messages = crate::core::conversation_persistence::list_messages_page(
        &pool,
        conversation.id,
        None,
        20,
    )
    .await?;

    assert!(messages.iter().any(|m| m.content_text.contains("Memory proposal resolved: Resolve me (rejected)")));
    assert!(messages.iter().any(|m| m.content_text.contains("Memory proposal 'Resolve me' was rejected.")));
    Ok(())
}
