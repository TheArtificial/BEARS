use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::core::{
    bears::{db, db::grant_membership, db::BearParams, BearProfile},
    tools::{
        arguments::DenToolChannelContext,
        constants::DEN_MEMORY_REQUEST_REVIEW,
        session::{invoke_den_tool, DenToolInvocationContext},
    },
    user::db::create_user,
};

async fn seed_pair_agent(
    pool: &PgPool,
    bear_id: Uuid,
    agent_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
        r#"
        INSERT INTO bear_profile_bindings (bear_id, profile, binding_id, letta_agent_id)
        VALUES ($1, 'pair', $2, $2)
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
async fn memory_request_review_projects_typed_conversation_records(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bear_id = db::create_bear(
        &pool,
        BearParams {
            slug: "test-memory-review-tool-bear",
            name: "Test Memory Review Tool Bear",
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
        &format!("mr-{}@ex.com", &suffix[..8]),
        &format!("mr{}", &suffix[..12]),
        "Memory Review Tester",
        "test-hash",
    )
    .await?;

    grant_membership(&pool, user_id, bear_id, Some("admin")).await?;

    let agent_id = format!("agent-{}", Uuid::new_v4());
    seed_pair_agent(&pool, bear_id, &agent_id).await?;

    let conversation = crate::core::conversation_persistence::ensure_conversation_for_external_id(
        &pool,
        bear_id,
        Some(user_id),
        "conv-memory-review-tool-test",
        None,
        None,
    )
    .await?;

    let context = DenToolInvocationContext {
        bear_id,
        bear_slug: "test-memory-review-tool-bear".to_string(),
        binding_id: agent_id,
        profile: Some(BearProfile::Pair),
        user_id,
        username: Some("tester".to_string()),
        membership_role: Some("owner".to_string()),
        conversation_id: "conv-memory-review-tool-test".to_string(),
        session_id: "acp-memory-review-tool-session".to_string(),
        acp_session_id: Some("acp-memory-review-tool-session".to_string()),
        conversation_selection: Some("conv-memory-review-tool-test".to_string()),
        runtime_target: None,
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        request_id: Some(Uuid::new_v4().to_string()),
        channel: DenToolChannelContext::default(),
    };

    let config = crate::config::Config::test_stub();
    let stores = crate::core::memory::MemoryStoreManager::new(&config);
    let payload = invoke_den_tool(
        &pool,
        &config,
        &stores,
        DEN_MEMORY_REQUEST_REVIEW,
        json!({
            "source_paths": ["pair/notes/test.md"],
            "title": "Promote memory",
            "summary": "Candidate memory summary",
            "suggested_action": "promote_to_core"
        }),
        context,
    )
    .await?;
    let proposal_id = payload["proposal"]["id"]
        .as_str()
        .expect("proposal id")
        .parse::<Uuid>()?;
    let context = crate::core::conversation_events::canonical_persistence_context(
        pool.clone(),
        bear_id,
        Some(user_id),
        "conv-memory-review-tool-test".to_string(),
        None,
        None,
        "acp-memory-review-tool-session".to_string(),
        false,
    );
    crate::core::conversation_events::persist_projection(
        &context,
        &crate::core::conversation_events::Projection {
            provenance: crate::core::conversation_events::ProjectionProvenance {
                source: crate::core::conversation_events::ProjectionSource::DenTools,
                scope_id: "acp-memory-review-tool-session".to_string(),
            },
            event: crate::core::conversation_events::ProjectionEvent::MemoryReviewRequested(
                crate::core::conversation_events::MemoryReviewRequestedPayload {
                    proposal_id,
                    source_profile: "pair".to_string(),
                    title: "Promote memory".to_string(),
                    suggested_action: "promote_to_core".to_string(),
                    status: "pending".to_string(),
                    source_paths: vec!["pair/notes/test.md".to_string()],
                },
            ),
            workflow_text: "Memory review requested: Promote memory".to_string(),
            visible_summary: Some(
                "Review requested for memory proposal 'Promote memory' from pair.".to_string(),
            ),
        },
    )
    .await?;

    assert_eq!(payload["proposal"]["title"], "Promote memory");

    let messages = crate::core::conversation_persistence::list_messages_page(
        &pool,
        conversation.id,
        None,
        20,
    )
    .await?;
    assert!(messages.iter().any(|m| m.content_text.contains("Memory review requested: Promote memory")));
    assert!(messages.iter().any(|m| m.content_text.contains("Review requested for memory proposal 'Promote memory' from pair.")));
    assert!(messages.iter().any(|m| m.message_type == "workflow_event"));
    Ok(())
}
