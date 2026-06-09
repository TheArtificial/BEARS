use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    core::{
        bears::{context_composition::compose_role_context, db as bears_db, model::BearAgentRole, Bear},
        llm::ChatMessage,
    },
    errors::CustomError,
};

use super::{
    context::load_transcript_messages,
    runtime_context::{
        assemble_den_owned_runtime_supplement, runtime_context_already_includes_den_owned_blocks,
    },
};

#[derive(Debug, Clone)]
pub struct AssembleTurnContext<'a> {
    pub pool: &'a PgPool,
    pub bear_id: Uuid,
    pub role: BearAgentRole,
    pub conversation_id: &'a str,
    pub turn_runtime_context: Option<&'a str>,
    pub human_message: Option<&'a str>,
    pub tool_messages: &'a [ChatMessage],
    pub session_id: Option<&'a str>,
    pub workspace_roots: Option<&'a [String]>,
    pub user_id: Option<i32>,
    pub client_context: Option<&'a serde_json::Value>,
    pub include_prompt_memory: bool,
}

impl<'a> AssembleTurnContext<'a> {
    pub fn should_load_den_owned_runtime_context(&self) -> bool {
        self.include_prompt_memory
            && self.session_id.is_some()
            && !self
                .turn_runtime_context
                .map(runtime_context_already_includes_den_owned_blocks)
                .unwrap_or(false)
    }
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
    } else if ctx.should_load_den_owned_runtime_context() {
        let session_id = ctx.session_id.expect("session_id checked above");
        let roots = ctx
            .workspace_roots
            .map(|items| items.to_vec())
            .unwrap_or_default();
        let client_context = ctx.client_context.cloned().unwrap_or_default();
        let supplement = assemble_den_owned_runtime_supplement(
            ctx.pool,
            ctx.bear_id,
            ctx.role.as_str(),
            session_id,
            &roots,
            &client_context,
        )
        .await?;
        if !supplement.trim().is_empty() {
            system_text.push_str("\n\n");
            system_text.push_str(&supplement);
        }
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
