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
    conversation_id: String,
    acp_session_id: String,
    finished: bool,
    pending_approval: Option<ApprovalPauseFuture>,
    pending_tool_event: Option<RuntimeStreamEvent>,
}

impl SessionTrackingStream {
    pub fn new(
        inner: Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>>,
        session: &AgentLoopSession,
        store: AgentLoopSessionStore,
        pool: sqlx::PgPool,
        bear_id: uuid::Uuid,
        conversation_id: String,
        acp_session_id: String,
    ) -> Self {
        Self {
            inner,
            session_key: session.session_key.clone(),
            store,
            assistant_text: String::new(),
            tool_calls: HashMap::new(),
            pool,
            bear_id,
            conversation_id,
            acp_session_id,
            finished: false,
            pending_approval: None,
            pending_tool_event: None,
        }
    }

    fn persist_assistant_tool_step(&self) {
        if self.tool_calls.is_empty() {
            return;
        }
        let calls: Vec<ChatToolCall> = self
            .tool_calls
            .iter()
            .map(|(id, (name, args))| ChatToolCall {
                id: id.clone(),
                call_type: "function".to_string(),
                function: crate::core::llm::ChatToolCallFunction {
                    name: name.clone(),
                    arguments: args.clone(),
                },
            })
            .collect();
        let content = if self.assistant_text.is_empty() {
            None
        } else {
            Some(self.assistant_text.clone())
        };
        self.store.update(&self.session_key, |session| {
            session.messages.push(crate::core::llm::ChatMessage {
                role: "assistant".to_string(),
                content,
                tool_call_id: None,
                name: None,
                tool_calls: Some(calls.clone()),
            });
            session.step += 1;
        });
    }
}

impl Stream for SessionTrackingStream {
    type Item = Result<RuntimeStreamEvent, CustomError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }

        if let Some(fut) = self.pending_approval.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Some(pause)) => {
                    self.pending_approval = None;
                    self.persist_assistant_tool_step();
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
                    self.persist_assistant_tool_step();
                }
                self.finished = true;
                Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                    RuntimeSemanticEvent::TurnCompleted { turn: None },
                ))))
            }
            Poll::Ready(other) => {
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
