use serde_json::json;
use std::collections::BTreeMap;

use crate::acp::AcpStreamContext;
use den_runtime::{
    gateway_events::{
            gateway_event_adapter_type, gateway_event_has_visible_output, GatewayEvent,
            ToolCallAccumulator,
        },
    turn_controller::TurnController,
};


pub(in crate::acp) fn classify_untranslated_provider_event(value: &serde_json::Value) -> String {
    value
        .get("message_type")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|message_type| format!("message_type:{message_type}"))
        .or_else(|| {
            value
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|event_type| format!("type:{event_type}"))
        })
        .or_else(|| value.as_object().map(|_| "object:unknown".to_string()))
        .or_else(|| value.as_array().map(|_| "array:unknown".to_string()))
        .or_else(|| value.as_str().map(|_| "string:unknown".to_string()))
        .unwrap_or_else(|| "value:unknown".to_string())
}

#[derive(Default)]
pub(in crate::acp) struct AcpStreamDiagnostics {
    pub(in crate::acp) upstream_frames: usize,
    pub(in crate::acp) parsed_events: usize,
    pub(in crate::acp) mapped_events: usize,
    pub(in crate::acp) unmapped_events: usize,
    pub(in crate::acp) native_message_types: BTreeMap<String, usize>,
    pub(in crate::acp) native_event_types: BTreeMap<String, usize>,
    pub(in crate::acp) adapter_event_types: BTreeMap<String, usize>,
    pub(in crate::acp) tool_request_counts: BTreeMap<String, usize>,
    pub(in crate::acp) tool_call_accumulator: ToolCallAccumulator,
    pub(in crate::acp) untranslated_event_classes: BTreeMap<String, usize>,
    pub(in crate::acp) unmapped_event_samples: Vec<String>,
    pub(in crate::acp) run_ids: Vec<String>,
    pub(in crate::acp) saw_visible_output: bool,
    /// True when the turn produced assistant text, a tool request, or runtime status/progress.
    /// Turn-controller chrome (planning/waiting status) and terminal markers do not count.
    pub(in crate::acp) saw_substantive_output: bool,
    pub(in crate::acp) saw_error: bool,
    pub(in crate::acp) saw_turn_complete: bool,
    pub(in crate::acp) saw_tool_return_ack: bool,
    pub(in crate::acp) saw_requires_approval_stop: bool,
    pub(in crate::acp) emitted_empty_turn_error: bool,
    pub(in crate::acp) emitted_runtime_cleanup: bool,
}

impl AcpStreamDiagnostics {
    pub(in crate::acp) fn resumed_continuation_defaults() -> Self {
        Self {
            saw_requires_approval_stop: false,
            ..Default::default()
        }
    }

    pub(in crate::acp) fn reset_for_resumed_continuation(&mut self) {
        self.saw_requires_approval_stop = false;
    }

    pub(in crate::acp) fn merge_from(&mut self, other: Self) {
        self.upstream_frames += other.upstream_frames;
        self.parsed_events += other.parsed_events;
        self.mapped_events += other.mapped_events;
        self.unmapped_events += other.unmapped_events;
        for (key, value) in other.native_message_types {
            *self.native_message_types.entry(key).or_insert(0) += value;
        }
        for (key, value) in other.native_event_types {
            *self.native_event_types.entry(key).or_insert(0) += value;
        }
        for (key, value) in other.adapter_event_types {
            *self.adapter_event_types.entry(key).or_insert(0) += value;
        }
        for (key, value) in other.tool_request_counts {
            *self.tool_request_counts.entry(key).or_insert(0) += value;
        }
        for (key, value) in other.untranslated_event_classes {
            *self.untranslated_event_classes.entry(key).or_insert(0) += value;
        }
        for sample in other.unmapped_event_samples {
            if self.unmapped_event_samples.len() < 5 {
                self.unmapped_event_samples.push(sample);
            }
        }
        for run_id in other.run_ids {
            if !self.run_ids.iter().any(|known| known == &run_id) {
                self.run_ids.push(run_id);
            }
        }
        self.saw_visible_output |= other.saw_visible_output;
        self.saw_substantive_output |= other.saw_substantive_output;
        self.saw_error |= other.saw_error;
        self.saw_turn_complete |= other.saw_turn_complete;
        self.saw_tool_return_ack |= other.saw_tool_return_ack;
        self.saw_requires_approval_stop |= other.saw_requires_approval_stop;
        self.emitted_empty_turn_error |= other.emitted_empty_turn_error;
        self.emitted_runtime_cleanup |= other.emitted_runtime_cleanup;
    }

    pub(in crate::acp) fn observe_runtime_event(
        &mut self,
        event: &den_runtime::runtime_provider::RuntimeStreamEvent,
    ) {
        self.parsed_events += 1;
        let runtime_type = match event {
            den_runtime::runtime_provider::RuntimeStreamEvent::UntranslatedProviderEvent { .. } => {
                "untranslated_provider_event"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::AssistantTextDelta { .. },
            ) => {
                self.saw_visible_output = true;
                self.saw_substantive_output = true;
                "assistant_text_delta"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::StatusText { .. },
            ) => {
                self.saw_visible_output = true;
                self.saw_substantive_output = true;
                "status_text"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::RunProgress { .. },
            ) => {
                self.saw_visible_output = true;
                self.saw_substantive_output = true;
                "run_progress"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::RunPaused { reason, .. },
            ) => {
                if reason == "awaiting_approval" || reason == "requires_approval" {
                    self.saw_requires_approval_stop = true;
                }
                "run_paused"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::ToolCallRequested {
                    tool_call_id,
                    ..
                },
            ) => {
                self.saw_substantive_output = true;
                let count = self.tool_request_counts.entry(tool_call_id.clone()).or_insert(0);
                *count += 1;
                "tool_call_requested"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::ToolCallFinished { .. },
            ) => {
                self.saw_visible_output = true;
                self.saw_substantive_output = true;
                "tool_call_finished"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::Error { .. },
            ) => {
                self.saw_error = true;
                self.saw_visible_output = true;
                "error"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::ConversationResolved {
                    conversation,
                },
            ) => {
                let run_id = conversation.id.clone();
                if !self.run_ids.iter().any(|known| known == &run_id) {
                    self.run_ids.push(run_id);
                }
                "conversation_resolved"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::TurnCompleted { .. },
            ) => {
                self.saw_turn_complete = true;
                "turn_completed"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::TurnFailed { .. },
            ) => {
                self.saw_error = true;
                self.saw_visible_output = true;
                "turn_failed"
            }
            den_runtime::runtime_provider::RuntimeStreamEvent::Semantic(
                den_runtime::runtime_provider::RuntimeSemanticEvent::TurnCancelled { .. },
            ) => {
                self.saw_error = true;
                self.saw_visible_output = true;
                "turn_cancelled"
            }
        };
        Self::increment(&mut self.native_event_types, runtime_type);
    }

    fn increment(map: &mut BTreeMap<String, usize>, key: &str) {
        let key = if key.trim().is_empty() { "<missing>" } else { key };
        *map.entry(key.to_string()).or_insert(0) += 1;
    }

    pub(in crate::acp) fn observe_parsed_event(&mut self, value: &serde_json::Value) -> Vec<String> {
        self.parsed_events += 1;
        let mut newly_observed_run_ids = Vec::new();
        let message_type = value.get("message_type").and_then(|v| v.as_str()).unwrap_or("");
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        for run_id in Self::extract_run_ids(value) {
            if self.observe_run_id(&run_id) {
                newly_observed_run_ids.push(run_id);
            }
        }
        Self::increment(&mut self.native_message_types, message_type);
        let stop_reason = value
            .get("stop_reason")
            .and_then(|v| v.as_str())
            .or_else(|| value.pointer("/message/stop_reason").and_then(|v| v.as_str()))
            .or_else(|| value.pointer("/data/stop_reason").and_then(|v| v.as_str()));
        if stop_reason == Some("requires_approval") {
            self.saw_requires_approval_stop = true;
        }
        Self::increment(&mut self.native_event_types, event_type);
        newly_observed_run_ids
    }

    fn extract_run_ids(value: &serde_json::Value) -> Vec<String> {
        let mut run_ids = Vec::new();
        for pointer in ["/run_id", "/message/run_id", "/data/run_id", "/run/id", "/message/run/id", "/data/run/id"] {
            if let Some(run_id) = value.pointer(pointer).and_then(serde_json::Value::as_str).map(str::trim).filter(|run_id| !run_id.is_empty()) {
                let run_id = run_id.to_string();
                if !run_ids.iter().any(|known| known == &run_id) {
                    run_ids.push(run_id);
                }
            }
        }
        for pointer in ["/run_ids", "/message/run_ids", "/data/run_ids"] {
            if let Some(items) = value.pointer(pointer).and_then(serde_json::Value::as_array) {
                for run_id in items.iter().filter_map(serde_json::Value::as_str).map(str::trim).filter(|run_id| !run_id.is_empty()) {
                    let run_id = run_id.to_string();
                    if !run_ids.iter().any(|known| known == &run_id) {
                        run_ids.push(run_id);
                    }
                }
            }
        }
        run_ids
    }

    fn observe_run_id(&mut self, run_id: &str) -> bool {
        let run_id = run_id.trim();
        if run_id.is_empty() || self.run_ids.iter().any(|known| known == run_id) {
            return false;
        }
        self.run_ids.push(run_id.to_string());
        true
    }

    pub(in crate::acp) fn observe_mapped_event(
        &mut self,
        event: &GatewayEvent,
        substantive: bool,
    ) {
        self.mapped_events += 1;
        Self::increment(&mut self.adapter_event_types, gateway_event_adapter_type(event));
        self.saw_visible_output |= gateway_event_has_visible_output(event);
        if substantive {
            self.saw_substantive_output |= Self::acp_event_is_substantive_output(event);
        }
        self.saw_error |= matches!(event, GatewayEvent::Error { .. });
        self.saw_turn_complete |= matches!(event, GatewayEvent::TurnComplete { .. } | GatewayEvent::TurnResult { .. });
    }

    fn acp_event_is_substantive_output(event: &GatewayEvent) -> bool {
        match event {
            GatewayEvent::AssistantTextDelta { text } | GatewayEvent::StatusText { text } => {
                !text.is_empty()
            }
            GatewayEvent::ToolRequest { .. } | GatewayEvent::Error { .. } => true,
            GatewayEvent::TurnComplete { .. }
            | GatewayEvent::TurnResult { .. }
            | GatewayEvent::PermissionRequest { .. }
            | GatewayEvent::PlanApprovalFallback { .. }
            | GatewayEvent::PlanUpdate { .. }
            | GatewayEvent::PlanUpdateJson { .. }
            | GatewayEvent::ModeUpdate { .. }
            | GatewayEvent::ConversationResolved { .. }
            | GatewayEvent::SessionInfoUpdate { .. } => false,
        }
    }

    pub(in crate::acp) fn observe_unmapped_event(&mut self, value: &serde_json::Value) {
        self.unmapped_events += 1;
        let class = classify_untranslated_provider_event(value);
        *self.untranslated_event_classes.entry(class).or_insert(0) += 1;
        if self.unmapped_event_samples.len() < 5 {
            self.unmapped_event_samples
                .push(super::logging::summarize_provider_event_for_log(value).to_string());
        }
    }

    pub(in crate::acp) fn empty_turn_error_event(&mut self, context: &AcpStreamContext) -> Option<GatewayEvent> {
        if self.emitted_empty_turn_error
            || self.saw_substantive_output
            || self.saw_error
            || self.saw_tool_return_ack
            || self.emitted_runtime_cleanup
        {
            return None;
        }
        self.emitted_empty_turn_error = true;
        let detail = format!(
            "Runtime stream ended without displayable assistant/status/error output. upstream_frames={}, parsed_events={}, mapped_events={}, unmapped_events={}, message_types={:?}, event_types={:?}",
            self.upstream_frames, self.parsed_events, self.mapped_events, self.unmapped_events, self.native_message_types, self.native_event_types,
        );
        Some(GatewayEvent::Error {
            message: "Runtime completed the turn without producing displayable ACP output.".to_string(),
            detail: Some(detail),
            error_type: Some("empty_mapped_turn".to_string()),
            request_id: Some(context.request_id.to_string()),
            context: Some(json!({
                "acp_session_id": context.acp_session_id,
                "unmapped_event_samples": self.unmapped_event_samples,
                "untranslated_event_classes": self.untranslated_event_classes,
                "run_ids": self.run_ids,
            })),
        })
    }

    pub(in crate::acp) fn mark_runtime_cleanup_emitted(&mut self) {
        self.emitted_runtime_cleanup = true;
    }

    pub(in crate::acp) fn diagnostic_json_with_turn_controller(&self, context: &AcpStreamContext, turn_controller: Option<&TurnController>) -> serde_json::Value {
        json!({
            "request_id": context.request_id,
            "acp_session_id": context.acp_session_id,
            "upstream_frames": self.upstream_frames,
            "parsed_events": self.parsed_events,
            "mapped_events": self.mapped_events,
            "unmapped_events": self.unmapped_events,
            "native_message_types": self.native_message_types,
            "native_event_types": self.native_event_types,
            "adapter_event_types": self.adapter_event_types,
            "tool_request_counts": self.tool_request_counts,
            "untranslated_event_classes": self.untranslated_event_classes,
            "run_ids": self.run_ids,
            "saw_visible_output": self.saw_visible_output,
            "saw_substantive_output": self.saw_substantive_output,
            "saw_error": self.saw_error,
            "saw_turn_complete": self.saw_turn_complete,
            "saw_tool_return_ack": self.saw_tool_return_ack,
            "saw_requires_approval_stop": self.saw_requires_approval_stop,
            "turn_controller": turn_controller.map(|controller| {
                let snapshot = controller.status_snapshot();
                json!({
                    "phase": format!("{:?}", snapshot.phase),
                    "open_obligations": snapshot.open_obligations,
                    "pending_adapter_tools": snapshot.pending_adapter_tools,
                    "pending_den_tools": snapshot.pending_den_tools,
                    "pending_permissions": snapshot.pending_permissions,
                    "terminal_status": snapshot.terminal_status.map(|status| format!("{:?}", status)),
                    "terminal_reason": snapshot.terminal_reason.map(|reason| format!("{:?}", reason)),
                    "orphaned_requires_approval": snapshot.orphaned_requires_approval,
                    "late_results_ignored": snapshot.late_results_ignored,
                })
            }),
        })
    }

    pub(in crate::acp) fn log_summary(&self, context: &AcpStreamContext) {
        let turn_result_count = self.adapter_event_types.get("turn_result").copied().unwrap_or(0);
        if turn_result_count > 1 {
            tracing::warn!(
                request_id = %context.request_id,
                acp_session_id = %context.acp_session_id,
                turn_result_count,
                "ACP stream emitted more than one terminal turn_result"
            );
        }
        tracing::info!(
            request_id = %context.request_id,
            acp_session_id = %context.acp_session_id,
            upstream_frames = self.upstream_frames,
            parsed_events = self.parsed_events,
            mapped_events = self.mapped_events,
            unmapped_events = self.unmapped_events,
            saw_visible_output = self.saw_visible_output,
            saw_substantive_output = self.saw_substantive_output,
            saw_error = self.saw_error,
            saw_turn_complete = self.saw_turn_complete,
            saw_tool_return_ack = self.saw_tool_return_ack,
            native_message_types = ?self.native_message_types,
            native_event_types = ?self.native_event_types,
            adapter_event_types = ?self.adapter_event_types,
            tool_request_counts = ?self.tool_request_counts,
            untranslated_event_classes = ?self.untranslated_event_classes,
            pending_tool_argument_buffers = self.tool_call_accumulator.pending_argument_buffers(),
            pending_tool_name_buffers = self.tool_call_accumulator.pending_name_buffers(),
            unmapped_event_samples = ?self.unmapped_event_samples,
            run_ids = ?self.run_ids,
            "ACP runtime stream summary"
        );
    }
}

