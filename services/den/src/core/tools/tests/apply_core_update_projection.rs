use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::core::{
    bears::{db, db::grant_membership, db::BearParams, BearProfile},
    memory_proposals::{create, CreateMemoryProposal},
    tools::{
        arguments::DenToolChannelContext,
        constants::DEN_MEMORY_APPLY_CORE_UPDATE,
        session::{invoke_den_tool, DenToolInvocationContext},
    },
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
async fn memory_apply_core_update_projects_typed_conversation_records(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bear_id = db::create_bear(
        &pool,
        BearParams {
            slug: "test-memory-apply-tool-bear",
            name: "Test Memory Apply Tool Bear",
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
        &format!("ac-{}@ex.com", &suffix[..8]),
        &format!("ac{}", &suffix[..12]),
        "Memory Apply Tester",
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
        "conv-memory-apply-tool-test",
        None,
        None,
    )
    .await?;

    let proposal = create(
        &pool,
        CreateMemoryProposal {
            bear_id,
            source_profile: BearProfile::Pair,
            source_agent_id: Some("agent-pair".to_string()),
            source_paths: vec!["pair/notes/test.md".to_string()],
            source_refs: json!({
                "conversation_id": "conv-memory-apply-tool-test",
                "session_id": "acp-memory-apply-tool-session"
            }),
            suggested_action: "promote_to_core",
            target_ref: None,
            title: "Apply me",
            summary: "candidate",
            rationale: "because",
            proposed_content: Some("Body text"),
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
        bear_slug: "test-memory-apply-tool-bear".to_string(),
        binding_id: agent_id,
        profile: Some(BearProfile::Curate),
        user_id,
        username: Some("tester".to_string()),
        membership_role: Some("admin".to_string()),
        conversation_id: "conv-memory-apply-tool-test".to_string(),
        session_id: "acp-memory-apply-tool-session".to_string(),
        acp_session_id: Some("acp-memory-apply-tool-session".to_string()),
        conversation_selection: Some("conv-memory-apply-tool-test".to_string()),
        runtime_target: None,
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        request_id: Some(Uuid::new_v4().to_string()),
        channel: DenToolChannelContext::default(),
    };

    let config = crate::config::Config {
        letta_memfs_service_url: "http://127.0.0.1:9".to_string(),
        ..crate::config::Config::test_stub()
    };
    let stores = crate::core::memory::MemoryStoreManager::new(&config);
    let payload = invoke_den_tool(
        &pool,
        &config,
        &stores,
        DEN_MEMORY_APPLY_CORE_UPDATE,
        json!({
            "proposal_id": proposal.id,
            "target_path": "core/notes.md",
            "mode": "append",
            "title": "Apply me"
        }),
        context,
    )
    .await;

    assert!(payload.is_err());

    let projection_context = crate::core::conversation_events::canonical_persistence_context(
        pool.clone(),
        bear_id,
        Some(user_id),
        "conv-memory-apply-tool-test".to_string(),
        None,
        None,
        "acp-memory-apply-tool-session".to_string(),
        false,
    );
    crate::core::conversation_events::persist_projection(
        &projection_context,
        &crate::core::conversation_events::memory_proposal_resolved_projection(
            crate::core::conversation_events::ProjectionProvenance {
                source: crate::core::conversation_events::ProjectionSource::DenTools,
                scope_id: "acp-memory-apply-tool-session".to_string(),
            },
            proposal.id,
            "pair".to_string(),
            "promote_to_core".to_string(),
            "Apply me".to_string(),
            "approved".to_string(),
            Some("curate".to_string()),
            Some("core/notes.md".to_string()),
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

    assert!(messages.iter().any(|m| m.content_text.contains("Memory proposal resolved: Apply me (approved)")));
    assert!(messages.iter().any(|m| m.content_text.contains("Memory proposal 'Apply me' was approved and applied at core/notes.md.")));
    Ok(())
}
