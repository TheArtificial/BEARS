use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    core::{
        bears::{
            db as bears_db, model::BearProfile, provision::profile_prompt_text, Bear,
        },
        memory::MemoryStoreManager,
        tools::work_surface::WorkSurfaceSessionHints,
        llm::ChatMessage,
    },
    errors::CustomError,
};

use super::{
    context::{
        load_transcript_messages, prune_messages_for_native_chat,
        prune_messages_for_native_pair, repair_tool_call_message_chain,
    },
    key_memory_projection::{
        project_key_memory, render_key_memory_projection_block, KeyMemoryProjectionCacheKey,
        KeyMemoryProjectionInput, KeyMemoryProjectionResult,
    },
    runtime_context::{
        assemble_den_owned_runtime_supplement, runtime_context_already_includes_den_owned_blocks,
    },
};

#[derive(Debug, Clone)]
pub struct AssembleTurnContext<'a> {
    pub pool: &'a PgPool,
    pub stores: &'a MemoryStoreManager,
    pub bear_id: Uuid,
    pub profile: BearProfile,
    pub conversation_id: &'a str,
    pub turn_runtime_context: Option<&'a str>,
    pub human_message: Option<&'a str>,
    pub tool_messages: &'a [ChatMessage],
    pub session_id: Option<&'a str>,
    pub workspace_roots: Option<&'a [String]>,
    pub runtime_target: Option<&'a str>,
    pub conversation_selection: Option<&'a str>,
    pub user_id: Option<i32>,
    pub client_context: Option<&'a serde_json::Value>,
    pub include_prompt_memory: bool,
    pub key_memory_cache: Option<&'a KeyMemoryProjectionCacheKey>,
    pub native_runtime: bool,
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

    fn session_hints(&self) -> WorkSurfaceSessionHints {
        WorkSurfaceSessionHints {
            runtime_target: self.runtime_target.map(str::to_string),
            conversation_selection: self.conversation_selection.map(str::to_string),
            workspace_roots: self
                .workspace_roots
                .map(|items| items.to_vec())
                .unwrap_or_default(),
        }
    }

    fn work_surface_status_override(&self) -> Option<&str> {
        self.client_context
            .and_then(|ctx| ctx.pointer("/work_surface/status"))
            .and_then(Value::as_str)
    }
}

#[derive(Debug, Clone)]
pub struct AssembledNativeTurn {
    pub messages: Vec<ChatMessage>,
    pub key_memory_projection: Option<KeyMemoryProjectionResult>,
}

pub async fn assemble_native_turn_messages(
    ctx: AssembleTurnContext<'_>,
) -> Result<Vec<ChatMessage>, CustomError> {
    Ok(assemble_native_turn(ctx).await?.messages)
}

pub async fn assemble_native_turn(
    ctx: AssembleTurnContext<'_>,
) -> Result<AssembledNativeTurn, CustomError> {
    let bear = bears_db::get_bear(ctx.pool, ctx.bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    assemble_native_turn_for_bear(ctx, &bear).await
}

pub async fn assemble_native_turn_messages_for_bear(
    ctx: AssembleTurnContext<'_>,
    bear: &Bear,
) -> Result<Vec<ChatMessage>, CustomError> {
    Ok(assemble_native_turn_for_bear(ctx, bear).await?.messages)
}

pub async fn assemble_native_turn_for_bear(
    ctx: AssembleTurnContext<'_>,
    bear: &Bear,
) -> Result<AssembledNativeTurn, CustomError> {
    let compiled_prompt = profile_prompt_text(ctx.pool, bear, ctx.profile).await?;
    let projection = if ctx.profile == BearProfile::Chat {
        KeyMemoryProjectionResult {
            rendered_text: String::new(),
            diagnostic: serde_json::json!({
                "source": "key_memory_projection",
                "status": "skipped_for_chat",
            }),
            cache_key: KeyMemoryProjectionCacheKey {
                bear_id: ctx.bear_id,
                profile: ctx.profile,
                conversation_id: ctx.conversation_id.to_string(),
                primary_surface_slug: None,
                sequence_high_water: 0,
                compiled_config_token: String::new(),
            },
        }
    } else {
        match project_key_memory(KeyMemoryProjectionInput {
        pool: ctx.pool,
        stores: ctx.stores,
        bear,
        profile: ctx.profile,
        conversation_id: ctx.conversation_id,
        session_hints: ctx.session_hints(),
        work_surface_status_override: ctx.work_surface_status_override(),
        native_runtime: ctx.native_runtime,
    })
    .await
    {
        Ok(projection) => projection,
        Err(err) => {
            tracing::warn!(
                bear_id = %ctx.bear_id,
                role = %ctx.profile.as_str(),
                conversation_id = %ctx.conversation_id,
                error = %err,
                "key memory projection failed; continuing without projected memory"
            );
            KeyMemoryProjectionResult {
                rendered_text: String::new(),
                diagnostic: serde_json::json!({
                    "source": "key_memory_projection",
                    "status": "error",
                    "error": err.to_string(),
                }),
                cache_key: KeyMemoryProjectionCacheKey {
                    bear_id: ctx.bear_id,
                    profile: ctx.profile,
                    conversation_id: ctx.conversation_id.to_string(),
                    primary_surface_slug: None,
                    sequence_high_water: 0,
                    compiled_config_token: String::new(),
                },
            }
        }
    }
    };
    if let Some(expected) = ctx.key_memory_cache {
        if &projection.cache_key != expected {
            tracing::debug!(
                bear_id = %ctx.bear_id,
                conversation_id = %ctx.conversation_id,
                "key memory projection cache key changed during turn assembly"
            );
        }
    }

    let mut system_text = compiled_prompt;
    if let Some(block) = render_key_memory_projection_block(&projection) {
        system_text.push_str("\n\n");
        system_text.push_str(&block);
    }
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
            ctx.profile.as_str(),
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
    if ctx.profile == BearProfile::Chat {
        system_text.push_str("\n\n");
        system_text.push_str(&crate::core::tools::descriptor::render_profile_tool_surface_blurb(
            ctx.profile,
        ));
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
    let messages = repair_tool_call_message_chain(messages);
    let messages = if ctx.native_runtime && ctx.profile == BearProfile::Pair {
        prune_messages_for_native_pair(messages)
    } else if ctx.native_runtime && ctx.profile == BearProfile::Chat {
        prune_messages_for_native_chat(messages)
    } else {
        messages
    };
    Ok(AssembledNativeTurn {
        messages,
        key_memory_projection: Some(projection),
    })
}
