use std::time::Instant;

use axum::http::HeaderMap;
use futures::StreamExt;
use serde_json::{json, Value};
use uuid::Uuid;

use den_core::tools::{
    constants::DEN_WEB_FETCH,
    result_compaction::{
        compact_client_tool_result_params, compact_client_tool_result_params_with_artifact,
    },
};
use den_http::{errors::CustomError, web_policy};
use den_runtime::{
    acp_sessions,
    bears::{db as bears_db, BearProfile},
    bearwire_events, bearwire_obligations, bearwire_runs,
    native_runtime::continue_native_acp_turn_event_stream,
    runtime::bearwire_projection::wire::BearWireEvent,
    runtime_contracts::{
        RoleRuntimeBinding, RuntimeApprovalDecision, RuntimeContinuation, RuntimeConversationRef,
        RuntimeToolResultStatus,
    },
    turn_runner::{default_tool_continue_stream_context, TurnContinueRequest},
    tool_output_artifacts::{create_tool_output_artifact, ToolOutputArtifactInput},
    DenState,
};

use crate::auth::authenticated_bear;
use crate::methods::run::{
    persist_run_failed, persist_run_progress, persist_runtime_event_as_bearwire,
};
use crate::methods::{param_string, required_param_string};

fn continuation_conversation_id(session: &acp_sessions::AcpSessionRow) -> String {
    session
        .resolved_conversation_id
        .clone()
        .unwrap_or_else(|| session.conversation_id.clone())
}

async fn record_web_fetch_approval_from_permission(
    pool: &sqlx::PgPool,
    bear_id: uuid::Uuid,
    user_id: i32,
    decision: &str,
    obligation_payload: &Value,
) -> Result<(), CustomError> {
    if !matches!(decision, "allow_once" | "allow_url" | "allow_host") {
        return Ok(());
    }
    let tool_name = obligation_payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(descriptor) =
        den_core::tools::descriptor::builtin_den_tool_descriptor_for_provider_name(tool_name)
    else {
        return Ok(());
    };
    if descriptor.name != DEN_WEB_FETCH {
        return Ok(());
    }
    let args = obligation_payload
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CustomError::ValidationError("web_fetch permission payload missing url".to_string())
        })?;
    let (scope_kind, scope_value) = if decision == "allow_host" {
        let host = match args
            .get("host")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(host) => web_policy::normalize_web_host(host)?,
            None => web_policy::normalize_web_url(url)?.host,
        };
        ("host", host)
    } else {
        ("url", web_policy::normalize_web_url(url)?.url)
    };
    let ttl_seconds = if decision == "allow_once" {
        Some(60 * 60)
    } else {
        None
    };
    web_policy::record_web_approval(
        pool,
        bear_id,
        scope_kind,
        &scope_value,
        Some(user_id),
        "acp",
        ttl_seconds,
    )
    .await?;
    Ok(())
}

fn continuation_unavailable_response(
    run: &bearwire_runs::BearWireRunRow,
    session_id: &str,
    conversation_id: &str,
    obligation_state: &str,
    obligation_id: impl ToString,
) -> Value {
    json!({
        "ok": false,
        "status": "continuation_unavailable",
        "reason": "native_agent_loop_session_not_found",
        "run_state": run.state,
        "obligation_state": obligation_state,
        "diagnostic": {
            "component": "den-bearwire",
            "phase": "native_session_missing_before_continuation",
            "run_id": run.run_id,
            "session_id": session_id,
            "conversation_id": conversation_id,
            "obligation_id": obligation_id.to_string(),
            "message": "Den cannot accept this client result for continuation because the in-memory native agent loop session is not present. This usually means Den restarted or the run was orphaned; retry the turn in a fresh session."
        }
    })
}

fn spawn_continuation_task(
    state: &DenState,
    run: bearwire_runs::BearWireRunRow,
    binding_id: String,
    conversation_id: String,
    continuation: RuntimeContinuation,
) {
    let pool = state.sqlx_pool.clone();
    let config = state.config.clone();
    let memory_stores = state.memory_stores.clone();
    tokio::spawn(async move {
        let continuation_started_at = Instant::now();
        let request_id = Uuid::new_v4();
        persist_run_progress(
            &pool,
            &run.session_id,
            &run.run_id,
            run.bear_id,
            run.user_id,
            continuation_started_at,
            "continuation_started",
            "Continuing Pair stance run after client result…",
            json!({
                "request_id": request_id,
            }),
        )
        .await;
        let _ = bearwire_runs::transition_run(
            &pool,
            &run.run_id,
            bearwire_runs::BearWireRunState::Continuing,
            None,
        )
        .await;
        let binding = RoleRuntimeBinding {
            binding_id,
            compatibility_backend: Some("native".to_string()),
        };
        let result = continue_native_acp_turn_event_stream(
            TurnContinueRequest {
                sqlx_pool: &pool,
                config: config.as_ref(),
                memory_stores: &memory_stores,
                request_id,
                run_id: Some(&run.run_id),
                acp_session_id: &run.session_id,
                conversation: RuntimeConversationRef {
                    id: conversation_id,
                },
                binding: &binding,
                continuation,
                stream_context: default_tool_continue_stream_context(),
            },
            BearProfile::Pair,
        )
        .await;
        match result {
            Ok((_continuation, mut stream)) => {
                persist_run_progress(
                    &pool,
                    &run.session_id,
                    &run.run_id,
                    run.bear_id,
                    run.user_id,
                    continuation_started_at,
                    "continuation_model_stream_waiting",
                    "Waiting for model output after local tool/permission result…",
                    json!({
                        "request_id": request_id,
                    }),
                )
                .await;
                let mut first_event_seen = false;
                let mut runtime_event_count = 0usize;
                let mut terminal_event_seen = false;
                let mut last_event_kind: Option<&'static str> = None;
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(runtime_event) => {
                            runtime_event_count += 1;
                            let event_kind = crate::methods::run::runtime_event_kind(&runtime_event);
                            last_event_kind = Some(event_kind);
                            if matches!(
                                &runtime_event,
                                den_protocol::RuntimeStreamEvent::Semantic(
                                    den_protocol::RuntimeSemanticEvent::TurnCompleted { .. }
                                        | den_protocol::RuntimeSemanticEvent::TurnFailed { .. }
                                        | den_protocol::RuntimeSemanticEvent::TurnCancelled { .. }
                                        | den_protocol::RuntimeSemanticEvent::Error { .. }
                                )
                            ) {
                                terminal_event_seen = true;
                            }
                            if !first_event_seen {
                                first_event_seen = true;
                                persist_run_progress(
                                    &pool,
                                    &run.session_id,
                                    &run.run_id,
                                    run.bear_id,
                                    run.user_id,
                                    continuation_started_at,
                                    "continuation_first_runtime_event",
                                    "Received first runtime event after continuation.",
                                    json!({
                                        "request_id": request_id,
                                        "event_kind": event_kind,
                                    }),
                                )
                                .await;
                            }
                            persist_runtime_event_as_bearwire(
                                &pool,
                                &run.session_id,
                                &run.run_id,
                                run.bear_id,
                                run.user_id,
                                runtime_event,
                                request_id,
                                Some(continuation_started_at),
                            )
                            .await;
                        }
                        Err(err) => {
                            persist_run_failed(
                                &pool,
                                &run.session_id,
                                &run.run_id,
                                run.bear_id,
                                run.user_id,
                                "continuation_stream_error",
                                err.to_string(),
                            )
                            .await;
                            break;
                        }
                    }
                }
                persist_run_progress(
                    &pool,
                    &run.session_id,
                    &run.run_id,
                    run.bear_id,
                    run.user_id,
                    continuation_started_at,
                    if terminal_event_seen {
                        "continuation_stream_ended_after_terminal"
                    } else {
                        "continuation_stream_ended_without_terminal"
                    },
                    if terminal_event_seen {
                        "Continuation stream ended after a terminal runtime event."
                    } else {
                        "Continuation stream ended without a terminal runtime event."
                    },
                    json!({
                        "request_id": request_id,
                        "runtime_event_count": runtime_event_count,
                        "first_event_seen": first_event_seen,
                        "terminal_event_seen": terminal_event_seen,
                        "last_event_kind": last_event_kind,
                    }),
                )
                .await;
            }
            Err(err) => {
                persist_run_failed(
                    &pool,
                    &run.session_id,
                    &run.run_id,
                    run.bear_id,
                    run.user_id,
                    "continuation_start_failed",
                    err.to_string(),
                )
                .await;
            }
        }
    });
}

pub(crate) async fn client_tool_result_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let run_id = required_param_string(params, "run_id")?;
    let session_id = required_param_string(params, "session_id")?;
    let tool_call_id = required_param_string(params, "tool_call_id")?;
    let status = param_string(params, "status").unwrap_or_else(|| "ok".to_string());
    let Some(run) = bearwire_runs::get_run(&state.sqlx_pool, &run_id).await? else {
        return Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "reason": "run_not_found",
        }));
    };
    if run.bear_id != bear.id || run.user_id != user_id || run.session_id != session_id {
        return Err(CustomError::Authorization(
            "run does not belong to authenticated Bear/session".to_string(),
        ));
    }
    if matches!(run.state.as_str(), "completed" | "failed" | "cancelled") {
        return Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "run_state": run.state,
            "reason": "run_is_terminal",
        }));
    }
    let obligation =
        bearwire_obligations::get_tool_call_obligation(&state.sqlx_pool, &run_id, &tool_call_id)
            .await?
            .ok_or_else(|| {
                CustomError::ValidationError(
                    "BearWire tool result has no persisted tool-call obligation".to_string(),
                )
            })?;
    if !bearwire_obligations::obligation_accepts_client_method(&obligation, "client.tool.result") {
        return Err(CustomError::ValidationError(format!(
            "BearWire tool obligation {} does not accept client.tool.result (expected {}, state {})",
            obligation.id,
            obligation.expected_client_method,
            obligation.state
        )));
    }
    let mut compacted = compact_client_tool_result_params(&tool_call_id, &status, params);
    if compacted.truncated {
        if let Ok(artifact) = create_tool_output_artifact(
            &state.sqlx_pool,
            ToolOutputArtifactInput {
                bear_id: bear.id,
                user_id: Some(user_id),
                session_id: session_id.clone(),
                conversation_id: None,
                run_id: Some(run_id.clone()),
                tool_call_id: tool_call_id.clone(),
                tool_name: params
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                source: "bearwire_client",
                content_text: params
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                content_json: Some(params.clone()),
                metadata: json!({ "status": status }),
            },
        )
        .await
        {
            compacted = compact_client_tool_result_params_with_artifact(
                &tool_call_id,
                &status,
                params,
                Some(&artifact.artifact_ref),
            );
        }
    }
    let payload = compacted.payload.clone();
    if !bearwire_obligations::obligation_is_open(&obligation) {
        return match bearwire_runs::existing_client_result_for_payload(
            &state.sqlx_pool,
            &run_id,
            "tool",
            &tool_call_id,
            &payload,
        )
        .await?
        {
            Some(bearwire_runs::BearWireClientResultRecord::DuplicateIdentical { row }) => {
                Ok(json!({
                    "ok": true,
                    "duplicate": true,
                    "result_id": row.id,
                    "run_state": run.state,
                    "obligation_state": obligation.state,
                }))
            }
            Some(bearwire_runs::BearWireClientResultRecord::DuplicateConflict { existing_hash }) => {
                Err(CustomError::ValidationError(format!(
                    "conflicting duplicate tool result for {tool_call_id}; existing hash {existing_hash}"
                )))
            }
            _ => Ok(json!({
                "ok": false,
                "status": "late_result_ignored",
                "run_state": run.state,
                "obligation_state": obligation.state,
            })),
        };
    }
    let session = acp_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let continuation_conversation_id = continuation_conversation_id(&session);
    if !den_runtime::native_runtime::native_acp_session_exists(
        &continuation_conversation_id,
        &session_id,
    ) {
        return Ok(continuation_unavailable_response(
            &run,
            &session_id,
            &continuation_conversation_id,
            &obligation.state,
            obligation.id,
        ));
    }
    let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, BearProfile::Pair)
        .await?
        .ok_or_else(|| CustomError::NotFound("Bear pair profile binding not found".to_string()))?;
    let record = bearwire_runs::record_client_result(
        &state.sqlx_pool,
        &run_id,
        "tool",
        &tool_call_id,
        payload.clone(),
    )
    .await?;
    match record {
        bearwire_runs::BearWireClientResultRecord::DuplicateConflict { existing_hash } => {
            Err(CustomError::ValidationError(format!(
                "conflicting duplicate tool result for {tool_call_id}; existing hash {existing_hash}"
            )))
        }
        bearwire_runs::BearWireClientResultRecord::DuplicateIdentical { row } => {
            Ok(json!({
                "ok": true,
                "duplicate": true,
                "result_id": row.id,
                "run_state": run.state,
            }))
        }
        bearwire_runs::BearWireClientResultRecord::Inserted { row } => {
            let Some(_received_obligation) = bearwire_obligations::mark_result_received(
                &state.sqlx_pool,
                obligation.id,
                payload.clone(),
            )
            .await? else {
                return Ok(json!({
                    "ok": false,
                    "status": "late_result_ignored",
                    "run_state": run.state,
                    "obligation_state": obligation.state,
                }));
            };
            let event_type = if status == "ok" {
                "tool_call.completed"
            } else {
                "tool_call.failed"
            };
            let mut event = BearWireEvent::ephemeral(event_type, payload);
            event.bear_id = Some(bear.id.to_string());
            event.human_id = Some(user_id.to_string());
            event.session_id = Some(session_id.clone());
            event.run_id = Some(run_id.clone());
            event.subject = Some(format!("resource/tool_call/{tool_call_id}"));
            let persisted = bearwire_events::append_bearwire_event(
                &state.sqlx_pool,
                &session_id,
                Some(bear.id),
                Some(user_id),
                event,
            )
            .await?;
            let transitioned = bearwire_runs::transition_run(
                &state.sqlx_pool,
                &run_id,
                bearwire_runs::BearWireRunState::Continuing,
                None,
            )
            .await?;
            let content = compacted.content.clone();
            let continuation_status = match status.as_str() {
                "ok" => RuntimeToolResultStatus::Ok,
                "timeout" | "timed_out" => RuntimeToolResultStatus::Timeout,
                _ => RuntimeToolResultStatus::Error,
            };
            spawn_continuation_task(
                state,
                transitioned.clone().unwrap_or(run.clone()),
                binding_id,
                continuation_conversation_id,
                RuntimeContinuation::ToolResult {
                    tool_call_id: tool_call_id.clone(),
                    approval_request_id: obligation.permission_id.clone(),
                    status: continuation_status,
                    content,
                },
            );
            let _ = bearwire_obligations::mark_continued(&state.sqlx_pool, obligation.id).await?;
            Ok(json!({
                "ok": true,
                "duplicate": false,
                "result_id": row.id,
                "event_sequence": persisted.sequence_no,
                "run_state": transitioned.map(|run| run.state).unwrap_or_else(|| "unknown".to_string()),
                "continuation": "started",
            }))
        }
    }
}

pub(crate) async fn client_permission_result_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let run_id = required_param_string(params, "run_id")?;
    let session_id = required_param_string(params, "session_id")?;
    let permission_id = required_param_string(params, "permission_id")?;
    let decision = param_string(params, "decision").unwrap_or_else(|| "denied".to_string());
    let Some(run) = bearwire_runs::get_run(&state.sqlx_pool, &run_id).await? else {
        return Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "reason": "run_not_found",
        }));
    };
    if run.bear_id != bear.id || run.user_id != user_id || run.session_id != session_id {
        return Err(CustomError::Authorization(
            "run does not belong to authenticated Bear/session".to_string(),
        ));
    }
    if matches!(run.state.as_str(), "completed" | "failed" | "cancelled") {
        return Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "run_state": run.state,
            "reason": "run_is_terminal",
        }));
    }
    let obligation =
        bearwire_obligations::get_permission_obligation(&state.sqlx_pool, &run_id, &permission_id)
            .await?
            .ok_or_else(|| {
                CustomError::ValidationError(
                    "BearWire permission result has no persisted permission obligation".to_string(),
                )
            })?;
    if !bearwire_obligations::obligation_accepts_client_method(
        &obligation,
        "client.permission.result",
    ) {
        return Err(CustomError::ValidationError(format!(
            "BearWire permission obligation {} does not accept client.permission.result (expected {}, state {})",
            obligation.id,
            obligation.expected_client_method,
            obligation.state
        )));
    }
    let normalized_decision = match decision.as_str() {
        "approved" | "approve" | "granted" | "allow" | "allow_once" | "allow_url"
        | "allow_host" => "granted",
        "denied" | "deny" | "rejected" | "reject" | "reject_once" | "reject_always" => "denied",
        "timeout" | "timed_out" => "expired",
        other => {
            return Err(CustomError::ValidationError(format!(
                "unsupported permission decision: {other}"
            )));
        }
    };
    let payload = json!({
        "permission_id": permission_id,
        "decision": normalized_decision,
        "reason": params.get("reason").cloned().unwrap_or(Value::Null),
    });
    if !bearwire_obligations::obligation_is_open(&obligation) {
        return match bearwire_runs::existing_client_result_for_payload(
            &state.sqlx_pool,
            &run_id,
            "permission",
            &permission_id,
            &payload,
        )
        .await?
        {
            Some(bearwire_runs::BearWireClientResultRecord::DuplicateIdentical { row }) => {
                Ok(json!({
                    "ok": true,
                    "duplicate": true,
                    "result_id": row.id,
                    "run_state": run.state,
                    "obligation_state": obligation.state,
                }))
            }
            Some(bearwire_runs::BearWireClientResultRecord::DuplicateConflict { existing_hash }) => {
                Err(CustomError::ValidationError(format!(
                    "conflicting duplicate permission result for {permission_id}; existing hash {existing_hash}"
                )))
            }
            _ => Ok(json!({
                "ok": false,
                "status": "late_result_ignored",
                "run_state": run.state,
                "obligation_state": obligation.state,
            })),
        };
    }
    let session = acp_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let continuation_conversation_id = continuation_conversation_id(&session);
    if !den_runtime::native_runtime::native_acp_session_exists(
        &continuation_conversation_id,
        &session_id,
    ) {
        return Ok(continuation_unavailable_response(
            &run,
            &session_id,
            &continuation_conversation_id,
            &obligation.state,
            obligation.id,
        ));
    }
    let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, BearProfile::Pair)
        .await?
        .ok_or_else(|| CustomError::NotFound("Bear pair profile binding not found".to_string()))?;
    let record = bearwire_runs::record_client_result(
        &state.sqlx_pool,
        &run_id,
        "permission",
        &permission_id,
        payload.clone(),
    )
    .await?;
    match record {
        bearwire_runs::BearWireClientResultRecord::DuplicateConflict { existing_hash } => {
            Err(CustomError::ValidationError(format!(
                "conflicting duplicate permission result for {permission_id}; existing hash {existing_hash}"
            )))
        }
        bearwire_runs::BearWireClientResultRecord::DuplicateIdentical { row } => {
            Ok(json!({
                "ok": true,
                "duplicate": true,
                "result_id": row.id,
                "run_state": run.state,
            }))
        }
        bearwire_runs::BearWireClientResultRecord::Inserted { row } => {
            let Some(_received_obligation) = bearwire_obligations::mark_result_received(
                &state.sqlx_pool,
                obligation.id,
                payload.clone(),
            )
            .await? else {
                return Ok(json!({
                    "ok": false,
                    "status": "late_result_ignored",
                    "run_state": run.state,
                    "obligation_state": obligation.state,
                }));
            };
            if normalized_decision == "granted" {
                record_web_fetch_approval_from_permission(
                    &state.sqlx_pool,
                    bear.id,
                    user_id,
                    decision.as_str(),
                    &obligation.request_payload,
                )
                .await?;
            }
            let event_type = match normalized_decision {
                "granted" => "permission.granted",
                "expired" => "permission.expired",
                _ => "permission.denied",
            };
            let mut event = BearWireEvent::ephemeral(event_type, payload);
            event.bear_id = Some(bear.id.to_string());
            event.human_id = Some(user_id.to_string());
            event.session_id = Some(session_id.clone());
            event.run_id = Some(run_id.clone());
            event.subject = Some(format!("resource/permission_request/{permission_id}"));
            let persisted = bearwire_events::append_bearwire_event(
                &state.sqlx_pool,
                &session_id,
                Some(bear.id),
                Some(user_id),
                event,
            )
            .await?;
            let transitioned = bearwire_runs::transition_run(
                &state.sqlx_pool,
                &run_id,
                bearwire_runs::BearWireRunState::Continuing,
                None,
            )
            .await?;
            let decision = if normalized_decision == "granted" {
                RuntimeApprovalDecision::Approve
            } else {
                RuntimeApprovalDecision::Deny
            };
            let reason = params
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            spawn_continuation_task(
                state,
                transitioned.clone().unwrap_or(run.clone()),
                binding_id,
                continuation_conversation_id,
                RuntimeContinuation::ApprovalDecision {
                    approval_request_id: permission_id.clone(),
                    tool_call_id: obligation.tool_call_id.clone(),
                    decision,
                    reason,
                },
            );
            let _ = bearwire_obligations::mark_continued(&state.sqlx_pool, obligation.id).await?;
            Ok(json!({
                "ok": true,
                "duplicate": false,
                "result_id": row.id,
                "event_sequence": persisted.sequence_no,
                "run_state": transitioned.map(|run| run.state).unwrap_or_else(|| "unknown".to_string()),
                "continuation": "started",
            }))
        }
    }
}
