use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    core::{
        bears::{context_composition::compose_role_context, db as bears_db, model::BearAgentRole, Bear},
        llm::ChatMessage,
    },
    errors::CustomError,
};

use super::context::load_transcript_messages;

#[derive(Debug, Clone)]
pub struct AssembleTurnContext<'a> {
    pub pool: &'a PgPool,
    pub bear_id: Uuid,
    pub role: BearAgentRole,
    pub conversation_id: &'a str,
    pub turn_runtime_context: Option<&'a str>,
    pub human_message: Option<&'a str>,
    pub tool_messages: &'a [ChatMessage],
}

pub async fn assemble_native_turn_messages(
    ctx: AssembleTurnContext<'_>,
) -> Result<Vec<ChatMessage>, CustomError> {
    let bear = bears_db::get_bear(ctx.pool, ctx.bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    assemble_native_turn_messages_for_bear(ctx, &bear).await
}

pub async fn assemble_native_turn_messages_for_bear(
    ctx: AssembleTurnContext<'_>,
    bear: &Bear,
) -> Result<Vec<ChatMessage>, CustomError> {
    let composed = compose_role_context(bear, ctx.role, None)?;
    let mut system_text = composed.composed_prompt;
    if let Some(runtime) = ctx
        .turn_runtime_context
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        system_text.push_str("\n\n");
        system_text.push_str(runtime);
    }
    let mut messages = vec![ChatMessage {
        role: "system".to_string(),
        content: Some(system_text),
        tool_call_id: None,
        name: None,
        tool_calls: None,
    }];
    messages.extend(load_transcript_messages(ctx.pool, ctx.bear_id, ctx.conversation_id).await?);
    if let Some(human) = ctx.human_message.map(str::trim).filter(|s| !s.is_empty()) {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: Some(human.to_string()),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        });
    }
    messages.extend(ctx.tool_messages.iter().cloned());
    Ok(messages)
}
