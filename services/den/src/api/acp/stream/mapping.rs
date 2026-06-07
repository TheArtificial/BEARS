use crate::{
    api::acp::{persist_stream_event_side_effects, AcpResolvedToolResult, AcpStreamContext},
    api::acp::types::PersistedToolRequestEffect,
    api::acp::stream::support::AcpStreamDiagnostics,
    core::{
        acp_letta_events::{
            map_native_letta_stream_event_to_acp_event_with_accumulator, AcpGatewayEvent,
        },
        runtime_bearwire_projection::runtime_semantic_event_to_bearwire_gateway_events,
        runtime_provider::{RuntimeSemanticEvent, RuntimeStreamEvent},
    },
};

pub(super) type AcpFrameResult = Result<
    (
        Vec<AcpGatewayEvent>,
        Option<PersistedToolRequestEffect>,
        Option<(String, String, AcpResolvedToolResult)>,
    ),
    std::io::Error,
>;

pub(in crate::api::acp) fn runtime_stream_event_to_acp_seed_value(
    runtime_event: RuntimeStreamEvent,
) -> Result<serde_json::Value, std::io::Error> {
    match runtime_event {
        RuntimeStreamEvent::UntranslatedProviderEvent { value } => Ok(value),
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            title,
            kind,
            arguments,
            approval_request_id,
            approval_required,
            approval_reason,
        }) => Ok(serde_json::json!({
            "message_type": if approval_required { "approval_request_message" } else { "tool_call_message" },
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "tool_title": title,
            "tool_kind": kind,
            "args": arguments,
            "approval_request_id": approval_request_id,
            "approval_reason": approval_reason,
        })),
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunPaused { reason, .. }) => {
            let stop_reason = if reason == "awaiting_approval" {
                "requires_approval".to_string()
            } else {
                reason
            };
            Ok(serde_json::json!({
                "message_type": "stop_reason",
                "stop_reason": stop_reason,
            }))
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. }) => {
            Ok(serde_json::json!({
                "message_type": "stop_reason",
                "stop_reason": "end_turn",
            }))
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { text }) => {
            Ok(serde_json::json!({
                "message_type": "assistant_message",
                "content": text,
            }))
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::StatusText { text }) => {
            Ok(serde_json::json!({
                "message_type": "reasoning_message",
                "reasoning": text,
            }))
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error {
            message,
            detail,
            error_type,
            request_id,
            context,
        }) => Ok(serde_json::json!({
            "message_type": "error_message",
            "message": message,
            "detail": detail,
            "error_type": error_type,
            "request_id": request_id,
            "context": context,
        })),
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ConversationResolved { conversation }) => {
            Ok(serde_json::json!({
                "type": "conversation_resolved",
                "conversation_id": conversation.id,
            }))
        }
        other => Err(std::io::Error::other(format!(
            "runtime event not supported by ACP persistence mapper: {other:?}"
        ))),
    }
}

pub(in crate::api::acp) async fn map_runtime_stream_event_to_acp_adapter_events_with_persistence(
    runtime_event: RuntimeStreamEvent,
    context: AcpStreamContext,
    diagnostics: &mut AcpStreamDiagnostics,
) -> AcpFrameResult {
    let runtime_event_for_projection = runtime_event.clone();
    let value = runtime_stream_event_to_acp_seed_value(runtime_event)?;
    let observed_run_ids = diagnostics.observe_parsed_event(&value);
    let direct_projected_events = match runtime_event_for_projection.clone() {
        RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::AssistantTextDelta { .. }
            | RuntimeSemanticEvent::StatusText { .. }
            | RuntimeSemanticEvent::ConversationResolved { .. }
            | RuntimeSemanticEvent::TurnCompleted { .. }
            | RuntimeSemanticEvent::TurnFailed { .. }
            | RuntimeSemanticEvent::TurnCancelled { .. }
            | RuntimeSemanticEvent::Error { .. }
            | RuntimeSemanticEvent::RunProgress { .. },
        ) => {
            if let RuntimeStreamEvent::Semantic(semantic_event) = runtime_event_for_projection.clone() {
                runtime_semantic_event_to_bearwire_gateway_events(semantic_event)
            } else {
                Vec::new()
            }
        }
        RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::ToolCallRequested { .. }
            | RuntimeSemanticEvent::RunPaused { .. },
        )
        | RuntimeStreamEvent::UntranslatedProviderEvent { .. } => Vec::new(),
    };
    if let Some(mut event) = map_native_letta_stream_event_to_acp_event_with_accumulator(
        &value,
        &mut diagnostics.tool_call_accumulator,
    ) {
        let mut tool_request_effect = persist_stream_event_side_effects(&context, &mut event)
            .await
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        let mut adapter_result_rx = None;
        let mut events = if let Some(effect) = tool_request_effect.as_mut() {
            match effect.route {
                crate::api::acp::ToolExecutionRoute::AdapterLocal => {
                    if let AcpGatewayEvent::ToolRequest { result_rx, .. } = &mut event {
                        if let Some(rx) = result_rx.take() {
                            let tool_call_id = effect.tool_call_id.clone();
                            let tool_name = effect.tool_name.clone();
                            adapter_result_rx = Some((
                                tool_call_id,
                                tool_name,
                                AcpResolvedToolResult::Receiver(rx),
                            ));
                        }
                    }
                }
                crate::api::acp::ToolExecutionRoute::DenServer => {
                    if let Some(rx) = effect.den_server_result_rx.take() {
                        let tool_call_id = effect.tool_call_id.clone();
                        let tool_name = effect.tool_name.clone();
                        adapter_result_rx = Some((
                            tool_call_id,
                            tool_name,
                            AcpResolvedToolResult::Receiver(rx),
                        ));
                    }
                }
                crate::api::acp::ToolExecutionRoute::Unsupported => {}
            }
            vec![event]
        } else {
            vec![event]
        };
        if !direct_projected_events.is_empty() {
            events.extend(direct_projected_events);
        }
        for run_id in observed_run_ids {
            if !diagnostics.run_ids.iter().any(|known| known == &run_id) {
                diagnostics.run_ids.push(run_id);
            }
        }
        Ok((events, tool_request_effect, adapter_result_rx))
    } else {
        diagnostics.observe_unmapped_event(&value);
        Ok((Vec::new(), None, None))
    }
}

#[cfg(test)]
pub(in crate::api::acp) fn summarize_event_for_log(value: &serde_json::Value) -> serde_json::Value {
    super::logging::summarize_letta_event_for_log(value)
}
