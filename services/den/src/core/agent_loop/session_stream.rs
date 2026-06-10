use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;

use crate::{
    core::{
        agent_loop::{
            session_store::AgentLoopSessionStore,
            tool_policy::{maybe_pause_for_tool_approval, provider_tool_requires_approval},
        },
        llm::ChatToolCall,
        runtime_contracts::{RuntimeSemanticEvent, RuntimeStreamEvent},
    },
    errors::CustomError,
};

use super::session_store::AgentLoopSession;
use super::transcript::spawn_persist_native_agent_step;

type ApprovalPauseFuture =
    Pin<Box<dyn Future<Output = Option<RuntimeSemanticEvent>> + Send>>;

pub struct SessionTrackingStream {
    inner: Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>>,
    session_key: String,
    store: AgentLoopSessionStore,
    assistant_text: String,
    tool_calls: HashMap<String, (String, String)>,
    pool: sqlx::PgPool,
    bear_id: uuid::Uuid,
    user_id: Option<i32>,
    conversation_id: String,
    acp_session_id: String,
    request_id: Option<String>,
    finished: bool,
    assistant_synced_to_session: bool,
    pending_approval: Option<ApprovalPauseFuture>,
    pending_tool_event: Option<RuntimeStreamEvent>,
    pending_pause_after_tool: Option<RuntimeSemanticEvent>,
}

impl SessionTrackingStream {
    pub fn new(
        inner: Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>>,
        session: &AgentLoopSession,
        store: AgentLoopSessionStore,
        pool: sqlx::PgPool,
        bear_id: uuid::Uuid,
        user_id: Option<i32>,
        conversation_id: String,
        acp_session_id: String,
        request_id: Option<String>,
    ) -> Self {
        Self {
            inner,
            session_key: session.session_key.clone(),
            store,
            assistant_text: String::new(),
            tool_calls: HashMap::new(),
            pool,
            bear_id,
            user_id,
            conversation_id,
            acp_session_id,
            request_id,
            finished: false,
            assistant_synced_to_session: false,
            pending_approval: None,
            pending_tool_event: None,
            pending_pause_after_tool: None,
        }
    }

    fn accumulated_tool_calls(&self) -> Vec<ChatToolCall> {
        self.tool_calls
            .iter()
            .map(|(id, (name, args))| ChatToolCall {
                id: id.clone(),
                call_type: "function".to_string(),
                function: crate::core::llm::ChatToolCallFunction {
                    name: name.clone(),
                    arguments: args.clone(),
                },
            })
            .collect()
    }

    fn assistant_content(&self) -> Option<String> {
        if self.assistant_text.is_empty() {
            None
        } else {
            Some(self.assistant_text.clone())
        }
    }

    fn sync_assistant_tool_step_to_session(&mut self) {
        if self.tool_calls.is_empty() {
            return;
        }
        let calls = self.accumulated_tool_calls();
        let content = self.assistant_content();
        let already_synced = self.assistant_synced_to_session;
        self.store.update(&self.session_key, |session| {
            upsert_assistant_tool_step_in_messages(
                &mut session.messages,
                content.clone(),
                &calls,
                already_synced,
            );
        });
        self.assistant_synced_to_session = true;
    }

    fn persist_assistant_tool_step(&self) {
        let calls = self.accumulated_tool_calls();
        spawn_persist_native_agent_step(
            self.pool.clone(),
            self.bear_id,
            self.user_id,
            self.conversation_id.clone(),
            self.acp_session_id.clone(),
            self.request_id.clone(),
            self.assistant_text.clone(),
            &calls,
        );
        if !self.tool_calls.is_empty() {
            self.store.update(&self.session_key, |session| {
                session.step += 1;
            });
            return;
        }
        if self.assistant_text.trim().is_empty() {
            return;
        }
        self.store.update(&self.session_key, |session| {
            session.messages.push(crate::core::llm::ChatMessage {
                role: "assistant".to_string(),
                content: self.assistant_content(),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            });
            session.step += 1;
        });
    }
}

fn upsert_assistant_tool_step_in_messages(
    messages: &mut Vec<crate::core::llm::ChatMessage>,
    content: Option<String>,
    calls: &[ChatToolCall],
    already_synced: bool,
) {
    let assistant = crate::core::llm::ChatMessage {
        role: "assistant".to_string(),
        content,
        tool_call_id: None,
        name: None,
        tool_calls: Some(calls.to_vec()),
    };
    if already_synced {
        if let Some(last) = messages.last_mut() {
            if last.role == "assistant" && last.tool_calls.is_some() {
                *last = assistant;
                return;
            }
        }
    }
    messages.push(assistant);
}

impl Stream for SessionTrackingStream {
    type Item = Result<RuntimeStreamEvent, CustomError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }

        if let Some(pause) = self.pending_pause_after_tool.take() {
            return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(pause))));
        }

        if let Some(fut) = self.pending_approval.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Some(pause)) => {
                    self.pending_approval = None;
                    self.persist_assistant_tool_step();
                    if let RuntimeSemanticEvent::RunPaused {
                        resume_token: Some(approval_id),
                        ..
                    } = &pause
                    {
                        if let Some(RuntimeStreamEvent::Semantic(
                            RuntimeSemanticEvent::ToolCallRequested {
                                approval_request_id,
                                ..
                            },
                        )) = self.pending_tool_event.as_mut()
                        {
                            *approval_request_id = Some(approval_id.clone());
                        }
                    }
                    if let Some(event) = self.pending_tool_event.take() {
                        self.pending_pause_after_tool = Some(pause);
                        return Poll::Ready(Some(Ok(event)));
                    }
                    self.finished = true;
                    return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(pause))));
                }
                Poll::Ready(None) => {
                    self.pending_approval = None;
                    if let Some(event) = self.pending_tool_event.take() {
                        return Poll::Ready(Some(Ok(event)));
                    }
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::AssistantTextDelta { text },
            )))) => {
                self.assistant_text.push_str(&text);
                if self.assistant_synced_to_session && !self.tool_calls.is_empty() {
                    self.sync_assistant_tool_step_to_session();
                }
                Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                    RuntimeSemanticEvent::AssistantTextDelta { text },
                ))))
            }
            Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::ToolCallRequested {
                    tool_call_id,
                    tool_name,
                    arguments,
                    ..
                },
            )))) => {
                self.tool_calls.insert(
                    tool_call_id.clone(),
                    (tool_name.clone(), arguments.to_string()),
                );
                self.sync_assistant_tool_step_to_session();
                let approval_required = provider_tool_requires_approval(&tool_name);
                let event = RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    title: None,
                    kind: Some("function".to_string()),
                    arguments: arguments.clone(),
                    approval_request_id: None,
                    approval_required,
                    approval_reason: if approval_required {
                        Some("native runtime policy".to_string())
                    } else {
                        None
                    },
                    run_id: None,
                });
                if approval_required {
                    let pool = self.pool.clone();
                    let bear_id = self.bear_id;
                    let conversation_id = self.conversation_id.clone();
                    let acp_session_id = self.acp_session_id.clone();
                    let tool_call_id = tool_call_id.clone();
                    let tool_name = tool_name.clone();
                    let arguments_value = arguments.clone();
                    self.pending_tool_event = Some(event);
                    self.pending_approval = Some(Box::pin(async move {
                        maybe_pause_for_tool_approval(
                            &pool,
                            bear_id,
                            &conversation_id,
                            &acp_session_id,
                            &tool_call_id,
                            &tool_name,
                            &arguments_value,
                        )
                        .await
                    }));
                    return Poll::Pending;
                }
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::TurnCompleted { .. },
            )))) => {
                if !self.tool_calls.is_empty() {
                    // Tool-call finishes must not emit TurnCompleted: ACP parks for adapter-local
                    // tool results and continues via /tool-results (same class of bug as
                    // openai_stream synthetic TurnCompleted).
                    self.persist_assistant_tool_step();
                    self.finished = true;
                    tracing::debug!(
                        acp_session_id = %self.acp_session_id,
                        tool_call_count = self.tool_calls.len(),
                        "native runtime suppressing TurnCompleted while tool calls are outstanding"
                    );
                    return Poll::Ready(None);
                }
                if !self.assistant_text.trim().is_empty() {
                    self.persist_assistant_tool_step();
                }
                self.finished = true;
                Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                    RuntimeSemanticEvent::TurnCompleted { turn: None },
                ))))
            }
            Poll::Ready(other) => {
                if matches!(other, None) && !self.tool_calls.is_empty() {
                    self.persist_assistant_tool_step();
                    self.finished = true;
                    tracing::debug!(
                        acp_session_id = %self.acp_session_id,
                        tool_call_count = self.tool_calls.len(),
                        "native runtime ended LLM stream with outstanding tool calls; deferring TurnCompleted"
                    );
                    return Poll::Ready(None);
                }
                if matches!(
                    other,
                    Some(Ok(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::TurnFailed { .. }
                    ))) | None
                ) {
                    self.finished = true;
                }
                Poll::Ready(other)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::llm::{ChatMessage, ChatToolCall, ChatToolCallFunction};

    fn sample_tool_call(id: &str) -> ChatToolCall {
        ChatToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: ChatToolCallFunction {
                name: "memory_read".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    #[test]
    fn upsert_assistant_tool_step_pushes_then_merges_tool_calls() {
        let mut messages = Vec::new();
        upsert_assistant_tool_step_in_messages(
            &mut messages,
            None,
            &[sample_tool_call("call_1")],
            false,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_calls.as_ref().map(|c| c.len()), Some(1));

        upsert_assistant_tool_step_in_messages(
            &mut messages,
            Some("checking".to_string()),
            &[sample_tool_call("call_1"), sample_tool_call("call_2")],
            true,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_deref(), Some("checking"));
        assert_eq!(messages[0].tool_calls.as_ref().map(|c| c.len()), Some(2));
    }

    #[test]
    fn assistant_tool_step_sync_precedes_tool_result_in_message_chain() {
        let mut messages = vec![ChatMessage {
            role: "user".to_string(),
            content: Some("hi".to_string()),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }];
        upsert_assistant_tool_step_in_messages(
            &mut messages,
            None,
            &[sample_tool_call("call_1")],
            false,
        );
        messages.push(ChatMessage {
            role: "tool".to_string(),
            content: Some("ok".to_string()),
            tool_call_id: Some("call_1".to_string()),
            name: None,
            tool_calls: None,
        });

        let repaired =
            crate::core::agent_loop::context::repair_tool_call_message_chain(messages);
        assert_eq!(repaired.len(), 3);
        assert_eq!(repaired[1].role, "assistant");
        assert_eq!(repaired[2].role, "tool");
    }
}
