use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::core::{
    conversation_events::{
        canonical_persistence_context, spawn_persist_assistant_output,
        spawn_persist_tool_request, spawn_persist_tool_result, ConversationEventProvenance,
    },
    llm::{ChatMessage, ChatToolCall},
};

use super::tool_policy::provider_tool_requires_approval;

pub fn spawn_persist_native_agent_step(
    pool: PgPool,
    bear_id: Uuid,
    user_id: Option<i32>,
    conversation_id: String,
    acp_session_id: String,
    request_id: Option<String>,
    assistant_text: String,
    tool_calls: &[ChatToolCall],
) {
    if assistant_text.trim().is_empty() && tool_calls.is_empty() {
        return;
    }
    let provenance = ConversationEventProvenance::acp_session(acp_session_id.clone());
    let context = canonical_persistence_context(
        pool,
        bear_id,
        user_id,
        conversation_id,
        Some(acp_session_id.clone()),
        request_id.clone(),
        acp_session_id.clone(),
        false,
    );
    if !assistant_text.trim().is_empty() {
        spawn_persist_assistant_output(
            context.clone(),
            assistant_text,
            &provenance,
            None,
            request_id.clone(),
        );
    }
    for call in tool_calls {
        let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| {
            Value::String(call.function.arguments.clone())
        });
        let approval_required = provider_tool_requires_approval(&call.function.name);
        spawn_persist_tool_request(
            context.clone(),
            call.function.name.clone(),
            call.id.clone(),
            request_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
            None,
            args,
            approval_required,
            if approval_required {
                Some("native runtime policy".to_string())
            } else {
                None
            },
            "native_runtime".to_string(),
            &provenance,
        );
    }
}

/// Persist a completed browser web chat turn after deferred step writes.
pub fn spawn_persist_web_chat_turn(
    pool: PgPool,
    bear_id: Uuid,
    user_id: i32,
    conversation_id: String,
    session_id: String,
    request_id: String,
    messages: &[ChatMessage],
    from_index: usize,
) {
    if from_index >= messages.len() {
        return;
    }
    let provenance = ConversationEventProvenance::acp_session(session_id.clone());
    let context = canonical_persistence_context(
        pool,
        bear_id,
        Some(user_id),
        conversation_id,
        Some(session_id.clone()),
        Some(request_id.clone()),
        session_id,
        false,
    );
    for message in messages.iter().skip(from_index) {
        match message.role.as_str() {
            "assistant" => {
                if let Some(text) = message
                    .content
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                {
                    spawn_persist_assistant_output(
                        context.clone(),
                        text.clone(),
                        &provenance,
                        None,
                        Some(request_id.clone()),
                    );
                }
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        let args: Value = serde_json::from_str(&call.function.arguments)
                            .unwrap_or_else(|_| Value::String(call.function.arguments.clone()));
                        let approval_required =
                            provider_tool_requires_approval(&call.function.name);
                        spawn_persist_tool_request(
                            context.clone(),
                            call.function.name.clone(),
                            call.id.clone(),
                            request_id.clone(),
                            None,
                            args,
                            approval_required,
                            if approval_required {
                                Some("native runtime policy".to_string())
                            } else {
                                None
                            },
                            "native_web_chat".to_string(),
                            &provenance,
                        );
                    }
                }
            }
            "tool" => {
                let Some(tool_call_id) = message.tool_call_id.clone() else {
                    continue;
                };
                spawn_persist_tool_result(
                    context.clone(),
                    message.name.clone(),
                    tool_call_id,
                    None,
                    "ok".to_string(),
                    message.content.clone(),
                    Value::Null,
                    serde_json::json!({
                        "component": "den.web_chat",
                        "phase": "server_side_tool_result",
                    }),
                    Some(request_id.clone()),
                    &provenance,
                );
            }
            _ => {}
        }
    }
}
