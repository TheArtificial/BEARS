//! Letta HTTP turn backend — retained only for `AGENT_RUNTIME=letta` escape hatch ([Phase 5](../../../docs/roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

use futures::{Stream, StreamExt};
use reqwest::Response;
use uuid::Uuid;

use crate::{
    api::service::ApiState,
    core::{
        acp_runtime::LettaRuntimeConversationBackend,
        acp_tool_turns::AcpToolTurnCoordinator,
        acp_turn_runner::{materialize_acp_runtime_conversation_if_needed, AcpTurnContinueRequest, AcpTurnStartRequest},
        letta::LettaClient,
        letta_runtime_stream_parser::{
            find_sse_frame_end, parse_sse_event_body_to_json, runtime_stream_event_from_letta_json,
            strip_trailing_sse_delimiter_owned,
        },
        pair_turn::{post_pair_turn_messages_streaming, PairTurnBoundaryLog, PairTurnRequest},
        runtime_contracts::{
            CancelTurnRequest, CancelTurnResult, ContinueTurnRequest,
            RuntimeApprovalDecision, RuntimeCancellationBackend, RuntimeCleanupRequest,
            RuntimeCleanupResult, RuntimeContinuation, RuntimeContinuationEnvelope,
            RuntimeConversationRef, RuntimeEventParser,
            RuntimeStreamContinuation, RuntimeToolResultStatus, StartTurnRequest,
        },
        runtime_conversations::{
            RuntimeApprovalActionMode, RuntimeApprovalActionRequest, RuntimeApprovalRequest,
            RuntimeConversationListRequest, RuntimeConversationMessagesRequest,
            RuntimeConversationSnapshot, RuntimePendingApproval,
        },
    },
    errors::CustomError,
};

pub struct LettaRuntimeCancellationBackend<'a> {
    letta: &'a LettaClient,
}

impl<'a> LettaRuntimeCancellationBackend<'a> {
    pub fn new(letta: &'a LettaClient) -> Self {
        Self { letta }
    }
}

#[allow(async_fn_in_trait)]
impl crate::core::runtime_conversations::RuntimeConversationBackend
    for LettaRuntimeCancellationBackend<'_>
{
    async fn list_conversations(
        &self,
        request: RuntimeConversationListRequest,
    ) -> Result<RuntimeConversationSnapshot, CustomError> {
        let _ = request.limit;
        Ok(crate::core::letta::load_agent_conversations(self.letta, &request.binding_id).await)
    }

    async fn list_messages(
        &self,
        request: RuntimeConversationMessagesRequest,
    ) -> Result<serde_json::Value, CustomError> {
        self.letta
            .list_conversation_messages(
                &request.conversation_id,
                request.binding_id.as_deref(),
                request.limit.try_into().map_err(|_| {
                    CustomError::ValidationError(
                        "conversation message limit exceeds u32".to_string(),
                    )
                })?,
                request.before.as_deref(),
                request.ascending,
            )
            .await
    }

    async fn pending_approvals(
        &self,
        request: RuntimeApprovalRequest,
    ) -> Result<Vec<RuntimePendingApproval>, CustomError> {
        let pending = self
            .letta
            .pending_conversation_approvals(
                &request.conversation_id,
                request.binding_id.as_deref(),
            )
            .await?;
        Ok(pending
            .into_iter()
            .map(|item| RuntimePendingApproval {
                tool_call_id: item.tool_call_id,
                approval_request_id: item.source_message_id,
                tool_name: item.name,
            })
            .collect())
    }

    async fn apply_approval_action(
        &self,
        request: RuntimeApprovalActionRequest,
    ) -> Result<Vec<RuntimePendingApproval>, CustomError> {
        let mode = match request.mode {
            RuntimeApprovalActionMode::InspectOnly => {
                crate::core::letta::PendingApprovalDenialMode::InspectOnly
            }
            RuntimeApprovalActionMode::Deny => {
                crate::core::letta::PendingApprovalDenialMode::PostToConversation
            }
        };
        let approvals = self
            .letta
            .deny_pending_conversation_approvals(
                &request.conversation_id,
                request.binding_id.as_deref(),
                &request.reason,
                mode,
            )
            .await?;
        Ok(approvals
            .into_iter()
            .map(|item| RuntimePendingApproval {
                tool_call_id: item.tool_call_id,
                approval_request_id: item.source_message_id,
                tool_name: item.name,
            })
            .collect())
    }
}

#[allow(async_fn_in_trait)]
impl RuntimeCancellationBackend for LettaRuntimeCancellationBackend<'_> {
    async fn cancel_turn(
        &self,
        request: CancelTurnRequest,
    ) -> Result<CancelTurnResult, CustomError> {
        let binding_id = request
            .binding
            .as_ref()
            .map(|binding| binding.binding_id.as_str())
            .unwrap_or("unknown-binding");
        let reason = request.reason.as_deref().unwrap_or("runtime_cancel");
        let run_ids = request.run_ids;
        if run_ids.is_empty() {
            tracing::warn!(
                pair_agent_id = binding_id,
                reason,
                "Skipping runtime run cancellation because no active run ids were recorded"
            );
            return Ok(CancelTurnResult {
                skipped: true,
                detail: "skipped:no_active_run_ids".to_string(),
            });
        }

        let url = format!(
            "{}/v1/agents/{binding_id}/messages/cancel",
            self.letta.base_url()
        );
        let body = serde_json::json!({ "run_ids": run_ids });
        let detail = match self.letta.http().post(url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => format!(
                "cancelled:{}",
                body["run_ids"].as_array().map(|ids| ids.len()).unwrap_or(0)
            ),
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                tracing::warn!(
                    pair_agent_id = binding_id,
                    reason,
                    run_ids = ?body["run_ids"],
                    %status,
                    body = %text,
                    "Failed runtime run cancellation request"
                );
                format!("failed:{status}:{text}")
            }
            Err(err) => {
                tracing::warn!(
                    pair_agent_id = binding_id,
                    reason,
                    run_ids = ?body["run_ids"],
                    error = %err,
                    "Failed runtime run cancellation request"
                );
                format!("failed:reqwest:{err}")
            }
        };
        Ok(CancelTurnResult {
            skipped: detail.starts_with("skipped:"),
            detail,
        })
    }

    async fn cleanup_stale_runtime(
        &self,
        request: RuntimeCleanupRequest,
    ) -> Result<RuntimeCleanupResult, CustomError> {
        let tool_turn_cleanup = request.acp_session_id.as_str().to_string();
        let cancel = self
            .cancel_turn(CancelTurnRequest {
                conversation: request.conversation.clone(),
                turn: None,
                reason: Some(request.reason.clone()),
                binding: Some(request.binding.clone()),
                run_ids: request.run_ids.clone(),
            })
            .await?;
        Ok(RuntimeCleanupResult {
            payload: serde_json::json!({
                "cancel": cancel.detail,
                "tool_turn_cleanup": tool_turn_cleanup,
                "run_ids": request.run_ids,
                "reason": request.reason,
                "request_id": request.request_id,
                "bear_id": request.bear_id,
                "pair_agent_id": request.binding.binding_id,
            }),
        })
    }
}

struct LettaRuntimeTurnBackend<'a> {
    letta: &'a LettaClient,
    request_id: Uuid,
    runtime_context_len: usize,
}

impl<'a> LettaRuntimeTurnBackend<'a> {
    fn new(letta: &'a LettaClient, request_id: Uuid, runtime_context_len: usize) -> Self {
        Self {
            letta,
            request_id,
            runtime_context_len,
        }
    }

    async fn post_turn_response(&self, request: &StartTurnRequest) -> Result<Response, CustomError> {
        let session_id = request
            .acp_session_id
            .as_deref()
            .ok_or_else(|| CustomError::ValidationError("missing acp_session_id".to_string()))?;
        post_pair_turn_messages_streaming(
            self.letta,
            PairTurnRequest {
                conversation_id: &request.conversation.id,
                binding_id: &request.binding.binding_id,
                human_message: &request.human_message,
                client_tools: request.client_tools.clone(),
                stream_tokens: request.stream_tokens,
                override_system: None,
                boundary: PairTurnBoundaryLog {
                    request_id: &self.request_id.to_string(),
                    channel_family: "acp",
                    session_id,
                    runtime_context_len: self.runtime_context_len,
                },
            },
        )
        .await
    }

    fn continuation_context(
        &self,
        conversation: &RuntimeConversationRef,
        binding: &crate::core::runtime_contracts::RoleRuntimeBinding,
    ) -> crate::core::letta::RuntimeContinuationContext {
        crate::core::letta::RuntimeContinuationContext {
            conversation_id: conversation.id.clone(),
            agent_id: Some(binding.binding_id.clone()),
            client_tools: None,
            stream_tokens: false,
            max_steps: 2,
        }
    }

    async fn continue_turn_response(
        &self,
        request: &ContinueTurnRequest,
    ) -> Result<Response, CustomError> {
        let session_id = request.conversation.id.as_str();
        let context = self.continuation_context(&request.conversation, &request.binding);
        match &request.continuation {
            RuntimeContinuation::ToolResult {
                tool_call_id,
                approval_request_id,
                status,
                content,
            } => {
                let status = match status {
                    RuntimeToolResultStatus::Ok => "ok",
                    RuntimeToolResultStatus::Error => "error",
                    RuntimeToolResultStatus::Timeout => "timeout",
                };
                let response = self
                    .letta
                    .post_conversation_tool_returns_streaming(
                        &context,
                        tool_call_id,
                        approval_request_id.as_deref(),
                        status,
                        content,
                    )
                    .await?;
                Ok(response)
            }
            RuntimeContinuation::ApprovalDecision {
                approval_request_id,
                tool_call_id,
                decision,
                reason,
            } => {
                let approve = matches!(decision, RuntimeApprovalDecision::Approve);
                let tool_call_id = tool_call_id.clone().unwrap_or_default();
                let content = if approve {
                    reason.clone().unwrap_or_else(|| "approved".to_string())
                } else {
                    reason.clone().unwrap_or_else(|| "denied".to_string())
                };
                let status = if approve { "ok" } else { "error" };
                let response = self
                    .letta
                    .post_conversation_tool_returns_streaming(
                        &context,
                        &tool_call_id,
                        Some(approval_request_id),
                        status,
                        &content,
                    )
                    .await?;
                let _ = session_id;
                Ok(response)
            }
        }
    }

    async fn start_turn_stream(
        &self,
        request: StartTurnRequest,
    ) -> Result<crate::core::runtime_contracts::RuntimeByteStream, CustomError> {
        let response = self.post_turn_response(&request).await?;
        Ok(Box::pin(
            response.bytes_stream().map(|item| item.map_err(Into::into)),
        ))
    }

    async fn continue_turn_stream(
        &self,
        request: ContinueTurnRequest,
    ) -> Result<crate::core::runtime_contracts::RuntimeByteStream, CustomError> {
        let response = self.continue_turn_response(&request).await?;
        Ok(Box::pin(
            response.bytes_stream().map(|item| item.map_err(Into::into)),
        ))
    }

    fn event_parser(&self) -> RuntimeEventParser {
        RuntimeEventParser {
            parse_json_event: runtime_stream_event_from_letta_json,
        }
    }
}

pub async fn start_letta_acp_turn_event_stream(
    request: AcpTurnStartRequest<'_>,
) -> Result<crate::core::runtime_contracts::RuntimeEventStream, CustomError> {
    let (bytes, parser) = start_letta_acp_turn_byte_stream(request).await?;
    Ok(runtime_byte_stream_to_event_stream(bytes, parser))
}

async fn start_letta_acp_turn_byte_stream(
    request: AcpTurnStartRequest<'_>,
) -> Result<
    (
        crate::core::runtime_contracts::RuntimeByteStream,
        RuntimeEventParser,
    ),
    CustomError,
> {
    let runtime_conversations = LettaRuntimeConversationBackend::new(request.state.letta.as_ref());
    let conversation_id = materialize_acp_runtime_conversation_if_needed(
        &runtime_conversations,
        &request,
    )
    .await?
    .conversation_id;
    let backend = LettaRuntimeTurnBackend::new(
        request.state.letta.as_ref(),
        request.request_id,
        request.runtime_context_len,
    );
    let parser = backend.event_parser();
    let stream = backend
        .start_turn_stream(StartTurnRequest {
            conversation: RuntimeConversationRef { id: conversation_id },
            binding: request.binding.clone(),
            human_message: request.prompt.to_string(),
            runtime_context: None,
            acp_session_id: Some(request.session_id.to_string()),
            client_tools: request.client_tools,
            stream_tokens: request.stream_tokens,
        })
        .await?;
    Ok((stream, parser))
}

pub async fn continue_letta_acp_turn_event_stream(
    request: AcpTurnContinueRequest<'_>,
) -> Result<
    (
        RuntimeStreamContinuation,
        crate::core::runtime_contracts::RuntimeEventStream,
    ),
    CustomError,
> {
    let status = match request.continuation {
        RuntimeContinuation::ToolResult { .. } | RuntimeContinuation::ApprovalDecision { .. } => {
            request.continuation
        }
    };
    let backend = LettaRuntimeTurnBackend::new(request.state.letta.as_ref(), request.request_id, 0);
    let parser = backend.event_parser();
    let stream = backend
        .continue_turn_stream(ContinueTurnRequest {
            conversation: request.conversation,
            turn: None,
            binding: request.binding.clone(),
            continuation: status,
        })
        .await?;
    let _envelope = RuntimeContinuationEnvelope {
        stream: RuntimeStreamContinuation::BytesSse,
        turn: None,
    };
    Ok((
        RuntimeStreamContinuation::BytesSse,
        runtime_byte_stream_to_event_stream(stream, parser),
    ))
}

pub fn runtime_byte_stream_to_event_stream(
    mut parsed: crate::core::runtime_contracts::RuntimeByteStream,
    parser: RuntimeEventParser,
) -> crate::core::runtime_contracts::RuntimeEventStream {
    use crate::core::runtime_contracts::{RuntimeSemanticEvent, RuntimeStreamEvent};
    let mut buffer = Vec::new();
    let mut queued_events: std::collections::VecDeque<
        Result<crate::core::runtime_contracts::RuntimeStreamEvent, CustomError>,
    > = std::collections::VecDeque::new();
    let mut finished = false;
    let mut saw_terminal_or_pause = false;
    let stream = futures::stream::poll_fn(move |cx| loop {
        if let Some(item) = queued_events.pop_front() {
            return std::task::Poll::Ready(Some(item));
        }
        if finished {
            return std::task::Poll::Ready(None);
        }
        match std::pin::Pin::new(&mut parsed).poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(bytes))) => {
                buffer.extend_from_slice(&bytes);
                while let Some(end) = find_sse_frame_end(&buffer) {
                    let raw: Vec<u8> = buffer.drain(..end).collect();
                    let frame_body = strip_trailing_sse_delimiter_owned(raw);
                    match parse_sse_event_body_to_json(&frame_body) {
                        Ok(Some(value)) => {
                            if let Some(event) = (parser.parse_json_event)(&value) {
                                if matches!(
                                    &event,
                                    RuntimeStreamEvent::Semantic(
                                        RuntimeSemanticEvent::RunPaused { .. }
                                            | RuntimeSemanticEvent::TurnCompleted { .. }
                                            | RuntimeSemanticEvent::TurnFailed { .. }
                                            | RuntimeSemanticEvent::TurnCancelled { .. }
                                            | RuntimeSemanticEvent::Error { .. }
                                    )
                                ) {
                                    saw_terminal_or_pause = true;
                                }
                                queued_events.push_back(Ok(event));
                            } else {
                                queued_events.push_back(Ok(
                                    RuntimeStreamEvent::UntranslatedProviderEvent { value },
                                ));
                            }
                        }
                        Ok(None) => {}
                        Err(err) => queued_events.push_back(Err(err)),
                    }
                }
            }
            std::task::Poll::Ready(Some(Err(err))) => {
                return std::task::Poll::Ready(Some(Err(err)));
            }
            std::task::Poll::Ready(None) => {
                finished = true;
                if buffer.is_empty() {
                    if !saw_terminal_or_pause {
                        queued_events.push_back(Ok(RuntimeStreamEvent::Semantic(
                            RuntimeSemanticEvent::TurnCompleted { turn: None },
                        )));
                    }
                } else {
                    queued_events.push_back(Err(CustomError::System(format!(
                        "continuation SSE stream ended with incomplete frame ({} bytes)",
                        buffer.len()
                    ))));
                }
            }
            std::task::Poll::Pending => return std::task::Poll::Pending,
        }
    });
    Box::pin(stream)
}

pub async fn letta_cleanup_stale_runtime_state(
    state: &ApiState,
    tool_turns: AcpToolTurnCoordinator,
    acp_session_id: String,
    bear_id: Uuid,
    pair_agent_id: String,
    run_ids: Vec<String>,
    reason: &'static str,
    request_id: Uuid,
) -> serde_json::Value {
    use crate::core::runtime_contracts::RoleRuntimeBinding;

    let tool_turn_cleanup = tool_turns.cleanup_request_tool_turns(&acp_session_id, request_id);
    let backend = LettaRuntimeCancellationBackend::new(state.letta.as_ref());
    match backend
        .cleanup_stale_runtime(RuntimeCleanupRequest {
            conversation: RuntimeConversationRef {
                id: acp_session_id.clone(),
            },
            binding: RoleRuntimeBinding {
                binding_id: pair_agent_id.clone(),
                compatibility_backend: Some("runtime:letta".to_string()),
            },
            acp_session_id: acp_session_id.clone(),
            bear_id,
            run_ids: run_ids.clone(),
            reason: reason.to_string(),
            request_id: request_id.to_string(),
        })
        .await
    {
        Ok(result) => serde_json::json!({
            "ok": result
                .payload
                .get("cancel")
                .and_then(serde_json::Value::as_str)
                .map(|detail| !detail.starts_with("failed:"))
                .unwrap_or(true),
            "reason": reason,
            "run_ids": run_ids,
            "cancel_result": result.payload.get("cancel").cloned().unwrap_or(serde_json::Value::Null),
            "tool_turn_cleanup": tool_turn_cleanup.to_json(),
            "cleanup_scope": {
                "kind": "request",
                "request_id": request_id,
            },
            "backend_cleanup": result.payload,
        }),
        Err(err) => serde_json::json!({
            "ok": false,
            "reason": reason,
            "run_ids": run_ids,
            "cancel_result": format!("failed:{err}"),
            "tool_turn_cleanup": tool_turn_cleanup.to_json(),
            "cleanup_scope": {
                "kind": "request",
                "request_id": request_id,
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::header,
        response::{IntoResponse, Response},
        routing::post,
        Json, Router,
    };
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;
    use tokio::sync::Mutex as TokioMutex;

    use crate::core::{
        acp_tool_turns::AcpToolTurnCoordinator,
        runtime_contracts::{RoleRuntimeBinding, RuntimeToolResultStatus},
    };

    #[derive(Clone)]
    struct FakeState {
        captured: Arc<TokioMutex<Option<serde_json::Value>>>,
    }

    async fn fake_tool_return(
        State(state): State<FakeState>,
        Json(body): Json<serde_json::Value>,
    ) -> Response {
        *state.captured.lock().await = Some(body);
        (
            [(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")],
            concat!(
                "data: {\"message_type\":\"assistant_message\",\"content\":\"continued\"}\n\n",
                "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"end_turn\"}\n\n"
            ),
        )
            .into_response()
    }

    fn test_api_state(letta: Arc<LettaClient>) -> ApiState {
        let config = Arc::new(crate::config::Config::test_stub());
        ApiState {
            sqlx_pool: PgPoolOptions::new()
                .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
                .unwrap(),
            config: config.clone(),
            letta,
            bifrost: Arc::new(crate::core::bifrost::BifrostClient::new(config.as_ref())),
            acp_tool_turns: AcpToolTurnCoordinator::new(),
            acp_turn_cancellations:
                crate::core::acp_turn_controller::AcpActiveTurnCancelRegistry::new(),
            memory_stores: crate::core::memory::MemoryStoreManager::new(config.as_ref()),
        }
    }

    #[tokio::test]
    async fn continue_turn_tool_result_posts_tool_return_payload() {
        let captured = Arc::new(TokioMutex::new(None));
        let app = Router::new()
            .route(
                "/v1/conversations/{conversation_id}/messages",
                post(fake_tool_return),
            )
            .with_state(FakeState {
                captured: captured.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let mut config = crate::config::Config::test_stub();
        config.letta_base_url = format!("http://{addr}");
        let letta = Arc::new(LettaClient::new(&config));
        let state = test_api_state(letta);
        let backend = LettaRuntimeTurnBackend::new(state.letta.as_ref(), Uuid::new_v4(), 0);

        let response = backend
            .continue_turn_stream(ContinueTurnRequest {
                conversation: RuntimeConversationRef {
                    id: "conv-test".to_string(),
                },
                turn: None,
                binding: RoleRuntimeBinding {
                    binding_id: "agent-test".to_string(),
                    compatibility_backend: Some("letta".to_string()),
                },
                continuation: RuntimeContinuation::ToolResult {
                    tool_call_id: "call-1".to_string(),
                    approval_request_id: None,
                    status: RuntimeToolResultStatus::Ok,
                    content: "plain tool result".to_string(),
                },
            })
            .await;
        assert!(response.is_ok());

        let body = captured.lock().await.clone().unwrap();
        assert_eq!(body["messages"][0]["type"], "tool_return");
        assert_eq!(body["messages"][0]["tool_returns"][0]["tool_call_id"], "call-1");
    }
}
