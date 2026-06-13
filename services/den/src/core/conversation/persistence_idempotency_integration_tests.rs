use sqlx::PgPool;

use crate::core::{
    bears::{db::create_bear, db::BearParams},
    conversation_message_types::{ConversationMessageRole, ConversationMessageType, ConversationMessageVisibility, ConversationMessageWrite},
    conversation_persistence::{append_message, ensure_conversation_for_external_id},
};

#[sqlx::test]
async fn duplicate_source_event_id_returns_existing_sequence(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bear_id = create_bear(
        &pool,
        BearParams {
            slug: "epic-a2-idempotency",
            name: "Epic A2 Idempotency",
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
        "conv-idempotency-test",
        None,
        None,
    )
    .await?;
    let source_event_id = "acp:assistant-output:req-123";
    let content_json = serde_json::json!({
        "event": "assistant_output",
        "request_id": "req-123"
    });

    let message = ConversationMessageWrite::structured(
        ConversationMessageType::Assistant,
        Some(ConversationMessageRole::Assistant),
        ConversationMessageVisibility::Default,
        "hello",
        content_json.clone(),
    )
    .with_source_event_id(Some(source_event_id.to_string()));
    let first = append_message(&pool, conversation.id, &message).await?;
    let second = append_message(
        &pool,
        conversation.id,
        &message.with_source_event_id(Some(source_event_id.to_string())),
    )
    .await?;

    assert_eq!(first, second);

    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint
        FROM conversation_messages
        WHERE conversation_id = $1
          AND source_event_id = $2
        "#,
    )
    .bind(conversation.id)
    .bind(source_event_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 1);

    Ok(())
}
