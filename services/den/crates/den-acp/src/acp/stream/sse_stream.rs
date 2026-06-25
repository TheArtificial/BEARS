use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{ready, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use futures::{Future, Stream};

use crate::{
    acp::{
        acp_debug_ui_enabled, acp_text_chunk_chars, acp_tool_result_followup_timeout_ms,
        acp_tool_timeout_ms_for_provider, continue_acp_turn_with_runtime,
        default_tool_continue_stream_context,
        looks_like_runtime_waiting_for_approval_error,
        map_runtime_stream_event_to_acp_adapter_events_with_persistence, mode_from_den_tool_result,
        plan_update_from_den_tool_result, AcpPendingFuture, AcpResolvedToolResult,
        AcpStaleRuntimeCleanupParams, AcpStreamContext, ActiveTurnCancelHandle, RoleRuntimeBinding,
        TurnContinueRequest,
    },
    core::tools::descriptor::den_tool_completion_status_text,
    service::DenState,
};
use den_http::errors::{CustomError, DenError};
use den_runtime::{
    agent_assist::normalize_display_status_text,
    bifrost::BifrostClient,
    gateway_events::{gateway_event_to_adapter_sse, GatewayEvent},
    role_runtime::{RoleTurnGuard, RoleTurnResult, TurnResultReason, TurnResultStatus},
    runtime_contracts::RuntimeConversationRef,
    runtime_provider::{RuntimeSemanticEvent, RuntimeStreamEvent},
    tool_turns::ToolResultRequest,
    turn_controller::{ActiveTurnCancelRegistry, TurnController, TurnPhase},
};

use super::{support::AcpStreamDiagnostics, text::AcpTextChunker};

/// Maximum silence before emitting a phase-aware status heartbeat to the adapter.
const ACP_STATUS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(6);

pub(in crate::acp) struct AcpRuntimeSseStream {
    pub(in crate::acp) inner:
        Pin<Box<dyn Stream<Item = Result<den_protocol::RuntimeStreamEvent, CustomError>> + Send>>,
    pub(in crate::acp) pending: VecDeque<Bytes>,
    pub(in crate::acp) context: AcpStreamContext,
    pub(in crate::acp) assistant_text_buffer: String,
    pub(in crate::acp) waiting_adapter_tool_result: Option<(String, String, AcpResolvedToolResult)>,
    pub(in crate::acp) queued_tool_result_continuation: Option<ToolResultRequest>,
    pub(in crate::acp) diagnostics: AcpStreamDiagnostics,
    pub(in crate::acp) logged_summary: bool,
    pub(in crate::acp) persist_future: Option<AcpPendingFuture>,
    pub(in crate::acp) session_info_event_sent: bool,
    pub(in crate::acp) text_chunker: AcpTextChunker,
    pub(in crate::acp) active_turn_guard: Option<RoleTurnGuard>,
    pub(in crate::acp) parked_adapter_result_rx: Option<(String, String, AcpResolvedToolResult)>,
    pub(in crate::acp) cancel_rx: Option<tokio::sync::watch::Receiver<bool>>,
    pub(in crate::acp) cancel_handle: Option<ActiveTurnCancelHandle>,
    pub(in crate::acp) turn_controller: TurnController,
    /// Last time any SSE frame was queued for the adapter (assistant text, tools, status, etc.).
    pub(in crate::acp) last_adapter_update_at: Instant,
    pub(in crate::acp) status_heartbeat_interval: Duration,
    status_heartbeat_sleep: Option<Pin<Box<tokio::time::Sleep>>>,
}

pub(in crate::acp) fn runtime_terminal_events(
    event: RuntimeStreamEvent,
    request_id: &str,
    acp_session_id: &str,
) -> Option<Vec<GatewayEvent>> {
    match event {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { message, .. }) => {
            Some(vec![
                GatewayEvent::Error {
                    message,
                    detail: None,
                    error_type: Some("runtime_turn_failed".to_string()),
                    request_id: Some(request_id.to_string()),
                    context: Some(serde_json::json!({
                        "component": "den.acp",
                        "acp_session_id": acp_session_id,
                    })),
                },
                GatewayEvent::TurnResult {
                    status: "failed".to_string(),
                    reason: "runtime_cleanup".to_string(),
                    request_id: Some(request_id.to_string()),
                    session_id: Some(acp_session_id.to_string()),
                    retryable: false,
                    diagnostics: serde_json::json!({
                        "component": "den.acp",
                        "source": "runtime_stream_event",
                        "event": "turn_failed",
                    }),
                },
            ])
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCancelled { .. }) => Some(vec![
            GatewayEvent::Error {
                message: "Runtime continuation was cancelled.".to_string(),
                detail: None,
                error_type: Some("runtime_turn_cancelled".to_string()),
                request_id: Some(request_id.to_string()),
                context: Some(serde_json::json!({
                    "component": "den.acp",
                    "acp_session_id": acp_session_id,
                })),
            },
            GatewayEvent::TurnResult {
                status: "cancelled".to_string(),
                reason: "cancelled".to_string(),
                request_id: Some(request_id.to_string()),
                session_id: Some(acp_session_id.to_string()),
                retryable: false,
                diagnostics: serde_json::json!({
                    "component": "den.acp",
                    "source": "runtime_stream_event",
                    "event": "turn_cancelled",
                }),
            },
        ]),
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error {
            message,
            detail,
            error_type,
            request_id: upstream_request_id,
            context: runtime_context,
        }) => {
            let terminal_request_id = upstream_request_id.unwrap_or_else(|| request_id.to_string());
            Some(vec![
                GatewayEvent::Error {
                    message,
                    detail,
                    error_type,
                    request_id: Some(terminal_request_id.clone()),
                    context: runtime_context.or_else(|| {
                        Some(serde_json::json!({
                            "component": "den.acp",
                            "acp_session_id": acp_session_id,
                        }))
                    }),
                },
                GatewayEvent::TurnResult {
                    status: "failed".to_string(),
                    reason: "runtime_cleanup".to_string(),
                    request_id: Some(terminal_request_id),
                    session_id: Some(acp_session_id.to_string()),
                    retryable: false,
                    diagnostics: serde_json::json!({
                        "component": "den.acp",
                        "source": "runtime_stream_event",
                        "event": "error",
                    }),
                },
            ])
        }
        _ => None,
    }
}

impl AcpRuntimeSseStream {
    pub(in crate::acp) fn outstanding_tool_obligations(&self) -> Vec<String> {
        self.context
            .tool_turns
            .pending_for_session(&self.context.acp_session_id)
            .into_iter()
            .filter(|turn| turn.request_id == self.context.request_id)
            .map(|turn| turn.tool_call_id)
            .collect()
    }

    pub(in crate::acp) fn controller_allows_terminal(&self) -> bool {
        self.turn_controller.may_emit_terminal()
    }

    pub(in crate::acp) fn turn_result_event(role_result: &RoleTurnResult) -> GatewayEvent {
        let terminal = role_result.to_terminal_event();
        GatewayEvent::TurnResult {
            status: terminal.status,
            reason: terminal.reason,
            request_id: terminal.request_id,
            session_id: terminal.session_id,
            retryable: terminal.retryable,
            diagnostics: terminal.diagnostics,
        }
    }

    pub(in crate::acp) fn push_adapter_event(&mut self, event: GatewayEvent) {
        self.enqueue_adapter_event(event, true);
    }

    fn enqueue_adapter_event(&mut self, event: GatewayEvent, substantive: bool) {
        if matches!(event, GatewayEvent::TurnComplete { .. })
            && !self.diagnostics.saw_substantive_output
        {
            // Upstream ended without assistant text or tool activity. Stream-end handling
            // emits empty_mapped_turn instead of accepting a bare terminal.
            return;
        }
        if matches!(event, GatewayEvent::TurnComplete { .. }) {
            self.turn_controller.on_stream_end();
            let Some(controller_terminal) = self.turn_controller.take_terminal_event() else {
                let snapshot = self.turn_controller.status_snapshot();
                tracing::info!(
                    request_id = %self.context.request_id,
                    acp_session_id = %self.context.acp_session_id,
                    controller_phase = ?snapshot.phase,
                    controller_open_obligations = snapshot.open_obligations,
                    "suppressed ACP turn_complete until turn controller allows terminal emission"
                );
                return;
            };
            tracing::debug!(
                request_id = %self.context.request_id,
                acp_session_id = %self.context.acp_session_id,
                controller_terminal_status = ?controller_terminal.status,
                controller_terminal_reason = ?controller_terminal.reason,
                "emitting ACP turn_complete authorized by turn controller"
            );
            self.persist_assistant_output_if_present();
        }
        if let GatewayEvent::AssistantTextDelta { text } = &event {
            self.assistant_text_buffer.push_str(text);
        }
        if matches!(event, GatewayEvent::SessionInfoUpdate { .. }) {
            self.session_info_event_sent = true;
        }
        self.diagnostics.observe_mapped_event(&event, substantive);
        self.pending.push_back(gateway_event_to_adapter_sse(event));
        self.record_adapter_update_emitted();
    }

    fn record_adapter_update_emitted(&mut self) {
        self.last_adapter_update_at = Instant::now();
        self.status_heartbeat_sleep = None;
    }

    fn should_emit_status_heartbeat(&self) -> bool {
        self.turn_controller.phase() != TurnPhase::Terminal
            && self.last_adapter_update_at.elapsed() >= self.status_heartbeat_interval
    }

    fn emit_status_heartbeat(&mut self) {
        let update = self.turn_controller.heartbeat_status_update();
        self.enqueue_adapter_event(GatewayEvent::StatusText { text: update.text }, false);
    }

    fn ensure_status_heartbeat_scheduled(&mut self) {
        if self.turn_controller.phase() == TurnPhase::Terminal {
            self.status_heartbeat_sleep = None;
            return;
        }
        if self.should_emit_status_heartbeat() {
            return;
        }
        if self.status_heartbeat_sleep.is_none() {
            let remaining = self
                .status_heartbeat_interval
                .saturating_sub(self.last_adapter_update_at.elapsed());
            self.status_heartbeat_sleep = Some(Box::pin(tokio::time::sleep(remaining)));
        }
    }

    fn poll_status_heartbeat(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<Bytes, std::io::Error>>> {
        if self.turn_controller.phase() == TurnPhase::Terminal {
            return Poll::Pending;
        }
        if self.should_emit_status_heartbeat() {
            self.emit_status_heartbeat();
            if let Some(bytes) = self.pending.pop_front() {
                return Poll::Ready(Some(Ok(bytes)));
            }
        }
        if let Some(sleep) = self.status_heartbeat_sleep.as_mut() {
            if sleep.as_mut().poll(cx).is_ready() {
                self.status_heartbeat_sleep = None;
                self.emit_status_heartbeat();
                if let Some(bytes) = self.pending.pop_front() {
                    return Poll::Ready(Some(Ok(bytes)));
                }
            }
        }
        Poll::Pending
    }

    pub(in crate::acp) fn persist_assistant_output_if_present(&mut self) {
        if self.assistant_text_buffer.is_empty() {
            return;
        }
        super::runtime::spawn_persist_acp_assistant_output(
            &self.context,
            std::mem::take(&mut self.assistant_text_buffer),
            None,
            Some(self.context.request_id.to_string()),
        );
    }

    pub(in crate::acp) fn persist_terminal_outcome(&mut self, role_result: &RoleTurnResult) {
        super::runtime::spawn_persist_acp_turn_outcome(&self.context, role_result);
    }

    pub(in crate::acp) fn push_terminal_result_now(&mut self, role_result: RoleTurnResult) {
        let Some(controller_terminal) = self.turn_controller.take_terminal_event() else {
            let snapshot = self.turn_controller.status_snapshot();
            tracing::warn!(
                request_id = %self.context.request_id,
                acp_session_id = %self.context.acp_session_id,
                controller_phase = ?snapshot.phase,
                controller_open_obligations = snapshot.open_obligations,
                controller_terminal_status = ?snapshot.terminal_status,
                controller_terminal_reason = ?snapshot.terminal_reason,
                "suppressed ACP turn_result because turn controller did not allow terminal emission"
            );
            return;
        };
        tracing::debug!(
            request_id = %self.context.request_id,
            acp_session_id = %self.context.acp_session_id,
            controller_terminal_status = ?controller_terminal.status,
            controller_terminal_reason = ?controller_terminal.reason,
            "emitting ACP turn_result authorized by turn controller"
        );
        self.persist_assistant_output_if_present();
        self.persist_terminal_outcome(&role_result);
        let event = Self::turn_result_event(&role_result);
        self.push_adapter_event(event);
    }

    pub(in crate::acp) fn push_terminal_result_when_ready(&mut self, role_result: RoleTurnResult) {
        if self.controller_allows_terminal() {
            self.push_terminal_result_now(role_result);
            return;
        }
        let controller_snapshot = self.turn_controller.status_snapshot();
        let outstanding = self.outstanding_tool_obligations();
        let pending_tool_continuation = self.queued_tool_result_continuation.is_some();
        tracing::warn!(
            request_id = %self.context.request_id,
            acp_session_id = %self.context.acp_session_id,
            outstanding_tool_call_ids = ?outstanding,
            pending_tool_continuation,
            controller_open_obligations = controller_snapshot.open_obligations,
            controller_phase = ?controller_snapshot.phase,
            controller_terminal_status = ?controller_snapshot.terminal_status,
            controller_terminal_reason = ?controller_snapshot.terminal_reason,
            "suppressed ACP turn_result because turn controller was not ready"
        );
    }

    pub(in crate::acp) fn new(
        inner: impl Stream<Item = Result<den_protocol::RuntimeStreamEvent, CustomError>>
            + Send
            + 'static,
        context: AcpStreamContext,
        initial_events: Vec<GatewayEvent>,
        session_info_event_sent: bool,
        active_turn_guard: RoleTurnGuard,
    ) -> Self {
        let mut pending = VecDeque::new();
        let mut last_adapter_update_at = Instant::now();
        for event in initial_events {
            pending.push_back(gateway_event_to_adapter_sse(event));
            last_adapter_update_at = Instant::now();
        }
        let mut turn_controller = TurnController::new();
        turn_controller.set_client_label(context.client.clone());
        turn_controller.on_stream_started();

        Self {
            inner: Box::pin(inner),
            pending,
            context,
            assistant_text_buffer: String::new(),
            waiting_adapter_tool_result: None,
            queued_tool_result_continuation: None,
            diagnostics: AcpStreamDiagnostics::default(),
            logged_summary: false,
            persist_future: None,
            session_info_event_sent,
            text_chunker: AcpTextChunker::new(acp_text_chunk_chars()),
            active_turn_guard: Some(active_turn_guard),
            parked_adapter_result_rx: None,
            cancel_rx: None,
            cancel_handle: None,
            turn_controller,
            last_adapter_update_at,
            status_heartbeat_interval: ACP_STATUS_HEARTBEAT_INTERVAL,
            status_heartbeat_sleep: None,
        }
    }

    #[cfg(test)]
    pub(in crate::acp) fn with_status_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.status_heartbeat_interval = interval;
        self
    }

    fn push_turn_status_update(&mut self) {
        if let Some(update) = self.turn_controller.take_status_update() {
            self.enqueue_adapter_event(GatewayEvent::StatusText { text: update.text }, false);
        }
    }

    #[cfg(test)]
    pub(in crate::acp) fn with_cancel_rx(
        mut self,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        self.cancel_rx = Some(cancel_rx);
        self
    }

    pub(in crate::acp) fn with_cancel_registration(
        mut self,
        handle: ActiveTurnCancelHandle,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        self.cancel_handle = Some(handle);
        self.cancel_rx = Some(cancel_rx);
        self
    }

    fn start_queued_tool_result_continuation(&mut self) -> bool {
        let Some(tool_result) = self.queued_tool_result_continuation.take() else {
            return false;
        };
        tracing::info!(
            request_id = %self.context.request_id,
            acp_session_id = %self.context.acp_session_id,
            tool_call_id = tool_result.tool_call_id.as_deref().unwrap_or("<missing>"),
            controller_phase = ?self.turn_controller.phase(),
            controller_open_obligations = self.turn_controller.status_snapshot().open_obligations,
            outstanding_tool_call_ids = ?self.outstanding_tool_obligations(),
            "ACP starting runtime continuation for queued tool result"
        );
        let prepared_continuation =
            match den_service::tool_turns::ToolTurnCoordinator::prepare_runtime_continuation(
                &tool_result,
            ) {
                Ok(prepared) => prepared,
                Err(
                    den_service::tool_turns::PrepareRuntimeContinuationError::MissingToolCallId {
                        display_tool_name,
                    },
                ) => {
                    self.pending.push_back(gateway_event_to_adapter_sse(
                    GatewayEvent::Error {
                        message: "Cannot continue runtime after ACP tool result without original tool_call_id.".to_string(),
                        detail: Some(format!(
                            "Tool result for {display_tool_name} did not include a tool_call_id; refusing to use tool name as a fallback."
                        )),
                        error_type: Some("missing_tool_call_id".to_string()),
                        request_id: Some(self.context.request_id.to_string()),
                        context: None,
                    },
                ));
                    return true;
                }
            };
        self.diagnostics.saw_tool_return_ack = true;
        let config = self.context.config.clone();
        let api_state = DenState {
            sqlx_pool: self.context.pool.clone(),
            config: config.clone(),
            bifrost: Arc::new(BifrostClient::new(config.as_ref())),
            bifrost_catalog: den_service::bifrost::new_catalog_store(),
            tool_turns: self.context.tool_turns.clone(),
            acp_turn_cancellations: ActiveTurnCancelRegistry::new(),
            memory_stores: self.context.memory_stores.clone(),
        };
        let binding = RoleRuntimeBinding {
            binding_id: self.context.pair_agent_id.clone(),
            compatibility_backend: Some("runtime:native".to_string()),
        };
        let request_id = self.context.request_id;
        let acp_session_id = self.context.acp_session_id.clone();
        let continuation_request = prepared_continuation.continuation;
        let stream_context = default_tool_continue_stream_context();
        let continuation_conversation = RuntimeConversationRef {
            id: self
                .context
                .resolved_conversation_id
                .clone()
                .unwrap_or_else(|| self.context.conversation_id.clone()),
        };
        self.persist_future = Some(AcpPendingFuture::ContinueTool(Box::pin(async move {
            let prepared = continue_acp_turn_with_runtime(TurnContinueRequest {
                sqlx_pool: &api_state.sqlx_pool,
                config: &api_state.config,
                memory_stores: &api_state.memory_stores,
                request_id,
                run_id: None,
                acp_session_id: &acp_session_id,
                conversation: continuation_conversation,
                binding: &binding,
                continuation: continuation_request,
                stream_context,
            })
            .await?;
            let diagnostics = AcpStreamDiagnostics::resumed_continuation_defaults();
            Ok((
                prepared.0,
                prepared.1,
                std::sync::Arc::new(std::sync::Mutex::new(diagnostics)),
            ))
        })));
        self.diagnostics.reset_for_resumed_continuation();
        true
    }

    pub(in crate::acp) fn cleanup_active_tool_turns(&mut self) {
        if self.turn_controller.phase() != TurnPhase::Terminal {
            return;
        }
        for pending in self
            .context
            .tool_turns
            .pending_for_session(&self.context.acp_session_id)
            .into_iter()
            .filter(|pending| pending.request_id == self.context.request_id)
        {
            self.context
                .tool_turns
                .remove(&self.context.acp_session_id, &pending.tool_call_id);
        }
    }

    pub(in crate::acp) fn log_summary_once(&mut self) {
        if !self.logged_summary {
            self.cleanup_active_tool_turns();
            self.diagnostics.log_summary(&self.context);
            self.logged_summary = true;
        }
    }
}

impl Drop for AcpRuntimeSseStream {
    fn drop(&mut self) {
        if let Some(waiting) = self.waiting_adapter_tool_result.take() {
            self.parked_adapter_result_rx = Some(waiting);
        }
        self.persist_assistant_output_if_present();
        self.cleanup_active_tool_turns();
    }
}

impl Stream for AcpRuntimeSseStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        if let Some(bytes) = this.pending.pop_front() {
            return Poll::Ready(Some(Ok(bytes)));
        }
        match this.poll_status_heartbeat(cx) {
            Poll::Ready(Some(item)) => return Poll::Ready(Some(item)),
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        if this.waiting_adapter_tool_result.is_none() {
            this.waiting_adapter_tool_result = this.parked_adapter_result_rx.take();
        }

        if this
            .cancel_rx
            .as_ref()
            .is_some_and(|cancel_rx| *cancel_rx.borrow())
            && this.turn_controller.phase() != TurnPhase::Terminal
        {
            this.turn_controller.on_cancel();
            let cancelled_tool_call_ids = this.outstanding_tool_obligations();
            for tool_call_id in &cancelled_tool_call_ids {
                this.context
                    .tool_turns
                    .remove(&this.context.acp_session_id, tool_call_id);
            }
            this.queued_tool_result_continuation = None;
            this.persist_future = None;
            let role_result = this.context.role_runtime.turn_result(
                TurnResultStatus::Cancelled,
                TurnResultReason::Cancelled,
                this.context.request_id,
                this.context.turn_scope.clone(),
                false,
                serde_json::json!({
                    "stream": this.diagnostics.diagnostic_json_with_turn_controller(&this.context, Some(&this.turn_controller)),
                    "cancelled_by": "acp_test_cancel_signal",
                }),
            );
            this.push_terminal_result_now(role_result);
            if let Some(bytes) = this.pending.pop_front() {
                return Poll::Ready(Some(Ok(bytes)));
            }
        }

        if this.turn_controller.phase() != TurnPhase::Terminal && this.persist_future.is_none() {
            if let Some((tool_call_id, tool_name, result_rx)) =
                this.waiting_adapter_tool_result.take()
            {
                let approval_request_id = this
                    .context
                    .tool_turns
                    .pending_for_session(&this.context.acp_session_id)
                    .into_iter()
                    .find(|pending| pending.tool_call_id == tool_call_id)
                    .and_then(|pending| pending.approval_request_id);
                let AcpResolvedToolResult::Receiver(result_rx) = result_rx;
                this.persist_future = Some(AcpPendingFuture::Tool(Box::pin(async move {
                    let timeout_ms = acp_tool_timeout_ms_for_provider(&tool_name);
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(timeout_ms),
                        result_rx,
                    )
                    .await
                    {
                        Err(_) => Some(Box::new(ToolResultRequest {
                            tool_call_id: Some(tool_call_id.clone()),
                            tool_name: Some(tool_name.clone()),
                            approval_request_id: approval_request_id.clone(),
                            status: "timeout".to_string(),
                            content: Some(format!(
                                "BEARS denied this approval automatically because `{tool_name}` timed out after {timeout_ms}ms."
                            )),
                            structured_content: serde_json::json!({}),
                            diagnostic: serde_json::json!({
                                "component": "den.acp",
                                "phase": "local_tool_result_timeout_auto_denied",
                                "tool_call_id": tool_call_id,
                                "tool_name": tool_name,
                                "timeout_ms": timeout_ms,
                            }),
                            ..Default::default()
                        })),
                        Ok(Err(err)) => Some(Box::new(ToolResultRequest {
                            tool_call_id: Some(tool_call_id.clone()),
                            tool_name: Some(tool_name.clone()),
                            approval_request_id: approval_request_id.clone(),
                            status: "error".to_string(),
                            content: Some(format!(
                                "BEARS denied this approval automatically because the ACP local tool result channel closed: {err}"
                            )),
                            structured_content: serde_json::json!({}),
                            diagnostic: serde_json::json!({
                                "component": "den.acp",
                                "phase": "local_tool_result_channel_closed_auto_denied",
                                "tool_call_id": tool_call_id,
                                "tool_name": tool_name,
                            }),
                            ..Default::default()
                        })),
                        Ok(Ok(value)) => Some(Box::new(value)),
                    }
                })));
                return self.poll_next(cx);
            }
        }

        if let Some(fut) = this.persist_future.as_mut() {
            match fut {
                AcpPendingFuture::Frame(fut) => {
                    let (result, diagnostics) = ready!(fut.as_mut().poll(cx));
                    this.persist_future = None;
                    this.diagnostics = diagnostics;
                    match result {
                        Ok((events, tool_effect, result_rx)) => {
                            for run_id in &this.diagnostics.run_ids {
                                let _ = this
                                    .cancel_handle
                                    .as_ref()
                                    .map(|handle| handle.record_run_id(run_id));
                            }
                            let has_tool_effect = tool_effect.is_some();
                            if let Some(effect) = tool_effect.as_ref() {
                                this.turn_controller.on_tool_request(
                                    effect.tool_call_id.clone(),
                                    effect.tool_name.clone(),
                                    effect.route.into(),
                                );
                            }
                            for event in events {
                                let continuation_terminal_failure = matches!(
                                    &event,
                                    GatewayEvent::Error { error_type: Some(error_type), .. }
                                        if error_type == "runtime_tool_result_followup_timeout"
                                            || error_type == "runtime_tool_result_missing_terminal"
                                );
                                for event in this.text_chunker.push(event) {
                                    this.push_adapter_event(event);
                                }
                                if continuation_terminal_failure
                                    && this.turn_controller.phase() != TurnPhase::Terminal
                                {
                                    this.turn_controller.on_stream_error();
                                    let role_result = this.context.role_runtime.turn_result(
                                        TurnResultStatus::Failed,
                                        TurnResultReason::RuntimeCleanup,
                                        this.context.request_id,
                                        this.context.turn_scope.clone(),
                                        false,
                                        serde_json::json!({
                                            "error": "runtime_tool_result_followup_missing_terminal",
                                            "stream": this.diagnostics.diagnostic_json_with_turn_controller(&this.context, Some(&this.turn_controller)),
                                        }),
                                    );
                                    this.push_terminal_result_when_ready(role_result);
                                }
                            }
                            if has_tool_effect {
                                this.push_turn_status_update();
                            }
                            if let Some((tool_call_id, tool_name, result_rx)) = result_rx {
                                this.waiting_adapter_tool_result =
                                    Some((tool_call_id, tool_name, result_rx));
                            }
                            return self.poll_next(cx);
                        }
                        Err(err) => {
                            let message = err.to_string();
                            tracing::warn!(
                                request_id = %this.context.request_id,
                                acp_session_id = %this.context.acp_session_id,
                                error = %message,
                                "ACP stream frame processing failed"
                            );
                            let event = GatewayEvent::Error {
                                message: "BEARS failed while processing an ACP stream event."
                                    .to_string(),
                                detail: Some(message),
                                error_type: Some("acp_stream_frame_processing_failed".to_string()),
                                request_id: Some(this.context.request_id.to_string()),
                                context: Some(serde_json::json!({
                                    "component": "den.acp",
                                    "acp_session_id": this.context.acp_session_id,
                                })),
                            };
                            this.push_adapter_event(event);
                            return self.poll_next(cx);
                        }
                    }
                }
                AcpPendingFuture::Tool(fut) => {
                    let result = ready!(fut.as_mut().poll(cx));
                    this.persist_future = None;
                    let Some(tool_result) = result else {
                        this.ensure_status_heartbeat_scheduled();
                        return Poll::Pending;
                    };
                    let tool_result = *tool_result;
                    {
                        if let Some(tool_call_id) = tool_result.tool_call_id.clone() {
                            super::runtime::spawn_persist_acp_tool_result(
                                &this.context,
                                tool_result.tool_name.clone(),
                                tool_call_id,
                                tool_result.approval_request_id.clone(),
                                tool_result.status.clone(),
                                tool_result.content.clone(),
                                tool_result.structured_content.clone(),
                                tool_result.diagnostic.clone(),
                                tool_result.request_id.clone(),
                            );
                        }
                        let settlement = this
                            .context
                            .tool_turns
                            .settle_after_result(&this.context.acp_session_id, &tool_result);
                        if let Some(done_id) = settlement.tool_call_id.as_deref() {
                            this.turn_controller
                                .on_adapter_tool_result(done_id, settlement.completed_ok);
                            if settlement.timed_out {
                                this.turn_controller.on_tool_timeout(done_id);
                            }
                            this.push_turn_status_update();
                        }
                        if let Some(plan_event) = plan_update_from_den_tool_result(&tool_result) {
                            this.push_adapter_event(plan_event);
                        }
                        if let Some(mode) = mode_from_den_tool_result(&tool_result) {
                            let mode_event = GatewayEvent::ModeUpdate {
                                mode: mode.to_string(),
                            };
                            this.push_adapter_event(mode_event);
                        }
                        let completion_text = normalize_display_status_text(
                            &if acp_debug_ui_enabled() {
                                format!(
                                    "BEARS debug: local tool {} completed with status {} ({} bytes)",
                                    settlement.display_tool_name,
                                    tool_result.status,
                                    tool_result.content.as_deref().map(str::len).unwrap_or(0),
                                )
                            } else {
                                den_tool_completion_status_text(&settlement.display_tool_name)
                                    .unwrap_or_else(|| {
                                        format!(
                                            "Local tool {} completed",
                                            settlement.display_tool_name
                                        )
                                    })
                            },
                        );
                        for event in this.text_chunker.flush_all() {
                            this.push_adapter_event(event);
                        }
                        this.push_adapter_event(GatewayEvent::StatusText {
                            text: completion_text,
                        });
                        tracing::info!(
                            request_id = %this.context.request_id,
                            acp_session_id = %this.context.acp_session_id,
                            tool_call_id = tool_result.tool_call_id.as_deref().unwrap_or("<missing>"),
                            controller_phase = ?this.turn_controller.phase(),
                            controller_open_obligations = this.turn_controller.status_snapshot().open_obligations,
                            outstanding_tool_call_ids = ?this.outstanding_tool_obligations(),
                            "ACP queued tool result continuation"
                        );
                        this.queued_tool_result_continuation = Some(tool_result);
                        return self.poll_next(cx);
                    }
                }
                AcpPendingFuture::ContinueTool(fut) => {
                    let result = ready!(fut.as_mut().poll(cx));
                    this.persist_future = None;
                    match result {
                        Ok((_continuation, stream, diagnostics)) => {
                            let context = this.context.clone();
                            let request_id = this.context.request_id.to_string();
                            let acp_session_id = this.context.acp_session_id.clone();
                            let diagnostics_for_stream = diagnostics.clone();
                            let mut runtime_stream = Box::pin(stream);
                            this.persist_future = Some(AcpPendingFuture::Frame(Box::pin(
                                async move {
                                    let mut queued_events = Vec::new();
                                    let mut saw_terminal_event = false;
                                    let followup_timeout = std::time::Duration::from_millis(
                                        acp_tool_result_followup_timeout_ms(),
                                    );
                                    let collect_followup = async {
                                        while let Some(item) =
                                            futures::StreamExt::next(&mut runtime_stream).await
                                        {
                                            match item {
                                                Ok(event) => {
                                                if let Ok(mut guard) = diagnostics_for_stream.lock()
                                                {
                                                    guard.observe_runtime_event(&event);
                                                }
                                                if let Some(events) = runtime_terminal_events(
                                                    event.clone(),
                                                    &request_id,
                                                    &acp_session_id,
                                                ) {
                                                    queued_events.extend(events);
                                                    saw_terminal_event = true;
                                                } else {
                                                    match event {
                                                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { text }) => {
                                                    queued_events.push(GatewayEvent::AssistantTextDelta { text });
                                                }
                                                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::StatusText { text }) => {
                                                    queued_events.push(GatewayEvent::StatusText { text });
                                                }
                                                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress { kind, text, phase: _, detail: _ }) => {
                                                    let rendered = if kind == "status_text" {
                                                        text.unwrap_or_default()
                                                    } else {
                                                        text.unwrap_or(kind)
                                                    };
                                                    queued_events.push(GatewayEvent::StatusText { text: rendered });
                                                }
                                                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallFinished { .. }) => {
                                                    if let RuntimeStreamEvent::Semantic(semantic) = event {
                                                        queued_events.extend(
                                                            den_runtime::runtime_bearwire_projection::runtime_semantic_event_to_bearwire_gateway_events(semantic),
                                                        );
                                                    }
                                                }
                                                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ConversationResolved { conversation }) => {
                                                    queued_events.push(GatewayEvent::ConversationResolved {
                                                        conversation_id: conversation.id,
                                                    });
                                                }
                                                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. }) => {
                                                    queued_events.push(GatewayEvent::TurnComplete {
                                                        outcome: "ok".to_string(),
                                                    });
                                                    saw_terminal_event = true;
                                                }
                                                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunPaused { .. })
                                                | RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested { .. })
                                                | RuntimeStreamEvent::UntranslatedProviderEvent { .. } => {
                                                    let mut temp_diagnostics = AcpStreamDiagnostics::default();
                                                    let (events, _effect, adapter_result_rx) = match map_runtime_stream_event_to_acp_adapter_events_with_persistence(
                                                        event,
                                                        context.clone(),
                                                        &mut temp_diagnostics,
                                                    ).await {
                                                        Ok(ok) => ok,
                                                        Err(err) => return Err(err),
                                                    };
                                                    if adapter_result_rx.is_some() {
                                                        // A continued runtime turn can immediately request another adapter-local tool.
                                                        // Do not allow the resumed stream collector to swallow that receiver; leave
                                                        // terminal emission to the outer stream, which owns the waiting/continuation loop.
                                                        break;
                                                    }
                                                    if events.iter().any(|event| matches!(event, GatewayEvent::TurnComplete { .. } | GatewayEvent::TurnResult { .. } | GatewayEvent::Error { .. })) {
                                                        saw_terminal_event = true;
                                                    }
                                                    if let Ok(mut guard) = diagnostics_for_stream.lock() {
                                                        guard.merge_from(temp_diagnostics);
                                                    }
                                                    queued_events.extend(events);
                                                }
                                                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { .. })
                                                | RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCancelled { .. })
                                                | RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error { .. }) => unreachable!(
                                                    "runtime terminal events are handled before non-terminal match"
                                                ),
                                                }
                                                }
                                                if saw_terminal_event {
                                                    break;
                                                }
                                                }
                                                Err(err) => return Err(std::io::Error::other(err.to_string())),
                                            }
                                        }
                                        Ok::<(), std::io::Error>(())
                                    };
                                    if tokio::time::timeout(followup_timeout, collect_followup)
                                        .await
                                        .is_err()
                                    {
                                        queued_events.push(GatewayEvent::Error {
                                            message: "Timed out waiting for the runtime to respond after an ACP local tool result.".to_string(),
                                            detail: Some(format!(
                                                "No assistant message or terminal event arrived within {}ms after the completed tool result was submitted.",
                                                followup_timeout.as_millis(),
                                            )),
                                            error_type: Some("runtime_tool_result_followup_timeout".to_string()),
                                            request_id: Some(request_id.clone()),
                                            context: Some(serde_json::json!({
                                                "component": "den.acp",
                                                "acp_session_id": acp_session_id,
                                                "timeout_ms": followup_timeout.as_millis(),
                                            })),
                                        });
                                        saw_terminal_event = true;
                                    }
                                    if !saw_terminal_event {
                                        queued_events.push(GatewayEvent::Error {
                                            message: "Tool finished but the runtime did not send a follow-up assistant response.".to_string(),
                                            detail: Some("The continuation stream ended after a completed tool result without assistant text or a terminal event.".to_string()),
                                            error_type: Some("runtime_tool_result_missing_terminal".to_string()),
                                            request_id: Some(request_id.clone()),
                                            context: Some(serde_json::json!({
                                                "component": "den.acp",
                                                "acp_session_id": acp_session_id,
                                            })),
                                        });
                                    }
                                    let mut diagnostics = std::sync::Arc::try_unwrap(diagnostics)
                                        .ok()
                                        .and_then(|m| m.into_inner().ok())
                                        .unwrap_or_default();
                                    for event in &queued_events {
                                        diagnostics.observe_mapped_event(event, true);
                                    }
                                    (Ok((queued_events, None, None)), diagnostics)
                                },
                            )));
                            return self.poll_next(cx);
                        }
                        Err(err) => {
                            let err: DenError = err.into();
                            if looks_like_runtime_waiting_for_approval_error(&err) {
                                let tool_turns = this.context.tool_turns.clone();
                                let acp_session_id = this.context.acp_session_id.clone();
                                let bear_id = this.context.bear_id;
                                let pair_agent_id = this.context.pair_agent_id.clone();
                                let run_ids = this.diagnostics.run_ids.clone();
                                let request_id = this.context.request_id;
                                // Cleanup-only state: `acp_cleanup_stale_runtime_state` never reads
                                // the model catalog, so an empty (unwarmed) store is intentional
                                // here. If catalog reads are ever added to this path, thread the
                                // shared store through `AcpStreamContext` instead of allocating one.
                                let cleanup_state = DenState {
                                    sqlx_pool: this.context.pool.clone(),
                                    config: this.context.config.clone(),
                                    bifrost: Arc::new(BifrostClient::new(
                                        this.context.config.as_ref(),
                                    )),
                                    bifrost_catalog: den_service::bifrost::new_catalog_store(),
                                    tool_turns: this.context.tool_turns.clone(),
                                    acp_turn_cancellations: ActiveTurnCancelRegistry::new(),
                                    memory_stores: this.context.memory_stores.clone(),
                                };
                                this.persist_future =
                                    Some(AcpPendingFuture::Cleanup(Box::pin(async move {
                                        super::super::acp_cleanup_stale_runtime_state(
                                            AcpStaleRuntimeCleanupParams {
                                                state: cleanup_state,
                                                tool_turns,
                                                acp_session_id,
                                                bear_id,
                                                pair_agent_id,
                                                run_ids,
                                                reason: "tool_return_continuation_failed",
                                                request_id,
                                            },
                                        )
                                        .await
                                    })));
                                return self.poll_next(cx);
                            }
                            this.pending.push_back(gateway_event_to_adapter_sse(
                                GatewayEvent::Error {
                                    message:
                                        "Failed to continue runtime after ACP local tool result."
                                            .to_string(),
                                    detail: Some(err.to_string()),
                                    error_type: Some("runtime_tool_return_failed".to_string()),
                                    request_id: Some(this.context.request_id.to_string()),
                                    context: None,
                                },
                            ));
                            return self.poll_next(cx);
                        }
                    }
                }
                AcpPendingFuture::Cleanup(fut) => {
                    let cleanup = ready!(fut.as_mut().poll(cx));
                    this.persist_future = None;
                    let reason = cleanup
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .map(|reason| {
                            if reason == "orphaned_requires_approval_stop" {
                                TurnResultReason::StaleApproval
                            } else {
                                TurnResultReason::RuntimeCleanup
                            }
                        })
                        .unwrap_or(TurnResultReason::RuntimeCleanup);
                    this.turn_controller.on_stream_end();
                    let role_result = this.context.role_runtime.turn_result(
                        TurnResultStatus::Recovered,
                        reason,
                        this.context.request_id,
                        this.context.turn_scope.clone(),
                        true,
                        serde_json::json!({
                            "cleanup": cleanup,
                            "stream": this.diagnostics.diagnostic_json_with_turn_controller(&this.context, Some(&this.turn_controller)),
                        }),
                    );
                    this.push_terminal_result_when_ready(role_result);
                    this.diagnostics.mark_runtime_cleanup_emitted();
                    return self.poll_next(cx);
                }
            }
        }

        if this.turn_controller.phase() == TurnPhase::Terminal {
            this.log_summary_once();
            this.cancel_handle.take();
            if let Some(guard) = this.active_turn_guard.take() {
                guard.release();
            }
            return Poll::Ready(None);
        }

        if this.pending.is_empty()
            && this.queued_tool_result_continuation.is_some()
            && this.outstanding_tool_obligations().is_empty()
            && this.persist_future.is_none()
            && this.start_queued_tool_result_continuation()
        {
            return self.poll_next(cx);
        }

        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                this.diagnostics.upstream_frames += 1;
                let context = this.context.clone();
                let mut diagnostics = std::mem::take(&mut this.diagnostics);
                this.persist_future = Some(AcpPendingFuture::Frame(Box::pin(async move {
                    let result = map_runtime_stream_event_to_acp_adapter_events_with_persistence(
                        event,
                        context,
                        &mut diagnostics,
                    )
                    .await;
                    (result, diagnostics)
                })));
                self.poll_next(cx)
            }
            Poll::Pending => {
                this.ensure_status_heartbeat_scheduled();
                Poll::Pending
            }
            Poll::Ready(None)
                if !this.outstanding_tool_obligations().is_empty()
                    || this.persist_future.is_some() =>
            {
                this.ensure_status_heartbeat_scheduled();
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(Some(Err(err))) => {
                let message = format!("Runtime stream read failed: {err}");
                tracing::warn!(
                    request_id = %this.context.request_id,
                    acp_session_id = %this.context.acp_session_id,
                    error = %err,
                    "ACP upstream runtime stream read error"
                );
                this.turn_controller.on_stream_error();
                let role_result = this.context.role_runtime.turn_result(
                    TurnResultStatus::Failed,
                    TurnResultReason::RuntimeCleanup,
                    this.context.request_id,
                    this.context.turn_scope.clone(),
                    false,
                    serde_json::json!({
                        "error": message,
                        "stream": this.diagnostics.diagnostic_json_with_turn_controller(&this.context, Some(&this.turn_controller)),
                    }),
                );
                this.push_terminal_result_when_ready(role_result);
                let event = serde_json::json!({
                    "type": "error",
                    "message": "Runtime stream ended unexpectedly while BEARS was waiting for events.",
                    "detail": message,
                    "request_id": this.context.request_id.to_string(),
                    "diagnostic": {
                        "code": "runtime_stream_read_error",
                        "component": "den.acp"
                    }
                });
                this.pending
                    .push_back(Bytes::from(format!("data: {}\n\n", event)));
                if let Some(bytes) = this.pending.pop_front() {
                    Poll::Ready(Some(Ok(bytes)))
                } else {
                    this.log_summary_once();
                    Poll::Ready(None)
                }
            }
            Poll::Ready(None) => {
                if this.diagnostics.saw_requires_approval_stop
                    && !this.outstanding_tool_obligations().is_empty()
                {
                    this.turn_controller.on_requires_approval_stop();
                    this.ensure_status_heartbeat_scheduled();
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else if this.queued_tool_result_continuation.is_none()
                    && !this.outstanding_tool_obligations().is_empty()
                {
                    tracing::debug!(
                        request_id = %this.context.request_id,
                        acp_session_id = %this.context.acp_session_id,
                        outstanding_tool_call_ids = ?this.outstanding_tool_obligations(),
                        "ACP upstream ended while local tool obligations are outstanding; waiting for results"
                    );
                    this.ensure_status_heartbeat_scheduled();
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else if let Some(tool_result) = this.queued_tool_result_continuation.take() {
                    tracing::info!(
                        request_id = %this.context.request_id,
                        acp_session_id = %this.context.acp_session_id,
                        tool_call_id = tool_result.tool_call_id.as_deref().unwrap_or("<missing>"),
                        controller_phase = ?this.turn_controller.phase(),
                        controller_open_obligations = this.turn_controller.status_snapshot().open_obligations,
                        outstanding_tool_call_ids = ?this.outstanding_tool_obligations(),
                        "ACP starting runtime continuation for queued tool result"
                    );
                    let prepared_continuation = match den_service::tool_turns::ToolTurnCoordinator::prepare_runtime_continuation(&tool_result) {
                        Ok(prepared) => prepared,
                        Err(den_service::tool_turns::PrepareRuntimeContinuationError::MissingToolCallId {
                            display_tool_name,
                        }) => {
                            this.pending.push_back(gateway_event_to_adapter_sse(
                                GatewayEvent::Error {
                                    message: "Cannot continue runtime after ACP tool result without original tool_call_id.".to_string(),
                                    detail: Some(format!(
                                        "Tool result for {display_tool_name} did not include a tool_call_id; refusing to use tool name as a fallback."
                                    )),
                                    error_type: Some("missing_tool_call_id".to_string()),
                                    request_id: Some(this.context.request_id.to_string()),
                                    context: None,
                                },
                            ));
                            return self.poll_next(cx);
                        }
                    };
                    this.diagnostics.saw_tool_return_ack = true;
                    let config = this.context.config.clone();
                    let api_state = DenState {
                        sqlx_pool: this.context.pool.clone(),
                        config: config.clone(),
                        bifrost: Arc::new(BifrostClient::new(config.as_ref())),
                        bifrost_catalog: den_service::bifrost::new_catalog_store(),
                        tool_turns: this.context.tool_turns.clone(),
                        acp_turn_cancellations: ActiveTurnCancelRegistry::new(),
                        memory_stores: this.context.memory_stores.clone(),
                    };
                    let binding = RoleRuntimeBinding {
                        binding_id: this.context.pair_agent_id.clone(),
                        compatibility_backend: Some("runtime:native".to_string()),
                    };
                    let request_id = this.context.request_id;
                    let acp_session_id = this.context.acp_session_id.clone();
                    let continuation_request = prepared_continuation.continuation;
                    let stream_context = default_tool_continue_stream_context();
                    let continuation_conversation = RuntimeConversationRef {
                        id: this
                            .context
                            .resolved_conversation_id
                            .clone()
                            .unwrap_or_else(|| this.context.conversation_id.clone()),
                    };
                    this.persist_future =
                        Some(AcpPendingFuture::ContinueTool(Box::pin(async move {
                            let prepared = continue_acp_turn_with_runtime(TurnContinueRequest {
                                sqlx_pool: &api_state.sqlx_pool,
                                config: &api_state.config,
                                memory_stores: &api_state.memory_stores,
                                request_id,
                                run_id: None,
                                acp_session_id: &acp_session_id,
                                conversation: continuation_conversation,
                                binding: &binding,
                                continuation: continuation_request,
                                stream_context,
                            })
                            .await?;
                            let diagnostics = AcpStreamDiagnostics::resumed_continuation_defaults();
                            Ok((
                                prepared.0,
                                prepared.1,
                                std::sync::Arc::new(std::sync::Mutex::new(diagnostics)),
                            ))
                        })));
                    this.diagnostics.reset_for_resumed_continuation();
                    self.poll_next(cx)
                } else if this.turn_controller.phase() == TurnPhase::WaitingForObligations
                    && this.turn_controller.status_snapshot().open_obligations == 0
                    && this.outstanding_tool_obligations().is_empty()
                    && !this.diagnostics.saw_tool_return_ack
                    && !this.diagnostics.emitted_runtime_cleanup
                    && this.queued_tool_result_continuation.is_none()
                {
                    let tool_turns = this.context.tool_turns.clone();
                    let acp_session_id = this.context.acp_session_id.clone();
                    let bear_id = this.context.bear_id;
                    let pair_agent_id = this.context.pair_agent_id.clone();
                    let run_ids = this.diagnostics.run_ids.clone();
                    let request_id = this.context.request_id;
                    let cleanup_state = DenState {
                        sqlx_pool: this.context.pool.clone(),
                        config: this.context.config.clone(),
                        bifrost: Arc::new(BifrostClient::new(this.context.config.as_ref())),
                        bifrost_catalog: den_service::bifrost::new_catalog_store(),
                        tool_turns: this.context.tool_turns.clone(),
                        acp_turn_cancellations: ActiveTurnCancelRegistry::new(),
                        memory_stores: this.context.memory_stores.clone(),
                    };
                    this.persist_future = Some(AcpPendingFuture::Cleanup(Box::pin(async move {
                        super::super::acp_cleanup_stale_runtime_state(
                            AcpStaleRuntimeCleanupParams {
                                state: cleanup_state,
                                tool_turns,
                                acp_session_id,
                                bear_id,
                                pair_agent_id,
                                run_ids,
                                reason: "orphaned_requires_approval_stop",
                                request_id,
                            },
                        )
                        .await
                    })));
                    self.poll_next(cx)
                } else if this.diagnostics.saw_requires_approval_stop
                    && this.outstanding_tool_obligations().is_empty()
                    && !this.diagnostics.emitted_runtime_cleanup
                    && this.queued_tool_result_continuation.is_none()
                {
                    let tool_turns = this.context.tool_turns.clone();
                    let acp_session_id = this.context.acp_session_id.clone();
                    let bear_id = this.context.bear_id;
                    let pair_agent_id = this.context.pair_agent_id.clone();
                    let run_ids = this.diagnostics.run_ids.clone();
                    let request_id = this.context.request_id;
                    let cleanup_state = DenState {
                        sqlx_pool: this.context.pool.clone(),
                        config: this.context.config.clone(),
                        bifrost: Arc::new(BifrostClient::new(this.context.config.as_ref())),
                        bifrost_catalog: den_service::bifrost::new_catalog_store(),
                        tool_turns: this.context.tool_turns.clone(),
                        acp_turn_cancellations: ActiveTurnCancelRegistry::new(),
                        memory_stores: this.context.memory_stores.clone(),
                    };
                    this.persist_future = Some(AcpPendingFuture::Cleanup(Box::pin(async move {
                        super::super::acp_cleanup_stale_runtime_state(
                            AcpStaleRuntimeCleanupParams {
                                state: cleanup_state,
                                tool_turns,
                                acp_session_id,
                                bear_id,
                                pair_agent_id,
                                run_ids,
                                reason: "requires_approval_without_tool_obligation",
                                request_id,
                            },
                        )
                        .await
                    })));
                    self.poll_next(cx)
                } else if let Some(event) = this.diagnostics.empty_turn_error_event(&this.context) {
                    for event in this.text_chunker.push(event) {
                        this.push_adapter_event(event);
                    }
                    self.poll_next(cx)
                } else if this.diagnostics.saw_error {
                    this.turn_controller.on_stream_error();
                    let role_result = this.context.role_runtime.turn_result(
                        TurnResultStatus::Failed,
                        TurnResultReason::RuntimeCleanup,
                        this.context.request_id,
                        this.context.turn_scope.clone(),
                        false,
                        this.diagnostics.diagnostic_json_with_turn_controller(
                            &this.context,
                            Some(&this.turn_controller),
                        ),
                    );
                    this.push_terminal_result_when_ready(role_result);
                    if !this.pending.is_empty() {
                        return self.poll_next(cx);
                    }
                    this.log_summary_once();
                    Poll::Ready(None)
                } else if this.diagnostics.saw_substantive_output {
                    for event in this.text_chunker.flush_all() {
                        this.push_adapter_event(event);
                    }
                    if this.turn_controller.phase() != TurnPhase::Terminal {
                        this.turn_controller.on_stream_end();
                        let compacted_retry =
                            den_runtime::native_runtime::take_session_overflow_compaction_recovered(
                                &this.context.conversation_id,
                                &this.context.acp_session_id,
                            );
                        let (status, reason, retryable) = if compacted_retry {
                            (
                                TurnResultStatus::Recovered,
                                TurnResultReason::CompactedRetry,
                                true,
                            )
                        } else {
                            (
                                TurnResultStatus::Ok,
                                TurnResultReason::StreamComplete,
                                false,
                            )
                        };
                        let role_result = this.context.role_runtime.turn_result(
                            status,
                            reason,
                            this.context.request_id,
                            this.context.turn_scope.clone(),
                            retryable,
                            this.diagnostics.diagnostic_json_with_turn_controller(
                                &this.context,
                                Some(&this.turn_controller),
                            ),
                        );
                        this.push_terminal_result_when_ready(role_result);
                    }
                    if !this.pending.is_empty() {
                        return self.poll_next(cx);
                    }
                    if this.turn_controller.phase() != TurnPhase::Terminal
                        && (!this.outstanding_tool_obligations().is_empty()
                            || this.waiting_adapter_tool_result.is_some()
                            || this.queued_tool_result_continuation.is_some())
                    {
                        this.ensure_status_heartbeat_scheduled();
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    this.log_summary_once();
                    Poll::Ready(None)
                } else {
                    tracing::warn!(
                        request_id = %this.context.request_id,
                        acp_session_id = %this.context.acp_session_id,
                        upstream_frames = this.diagnostics.upstream_frames,
                        native_event_types = ?this.diagnostics.native_event_types,
                        "ACP stream ended without substantive output; emitting failed turn_result"
                    );
                    this.turn_controller.on_stream_error();
                    let role_result = this.context.role_runtime.turn_result(
                        TurnResultStatus::Failed,
                        TurnResultReason::RuntimeCleanup,
                        this.context.request_id,
                        this.context.turn_scope.clone(),
                        false,
                        this.diagnostics.diagnostic_json_with_turn_controller(
                            &this.context,
                            Some(&this.turn_controller),
                        ),
                    );
                    this.push_terminal_result_when_ready(role_result);
                    if !this.pending.is_empty() {
                        return self.poll_next(cx);
                    }
                    this.log_summary_once();
                    Poll::Ready(None)
                }
            }
        }
    }
}
