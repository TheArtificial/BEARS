use std::time::Instant;

use axum::http::HeaderMap;
use futures::StreamExt;
use serde_json::{json, Value};
use uuid::Uuid;

use den_http::errors::CustomError;
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
    DenState,
};

use crate::auth::authenticated_bear;
use crate::methods::run::{
    persist_run_failed, persist_run_progress, persist_runtime_event_as_bearwire,
};
use crate::methods::{param_string, required_param_string};

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
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(runtime_event) => {
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
                                        "event_kind": crate::methods::run::runtime_event_kind(&runtime_event),
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
    let payload = json!({
        "tool_call_id": tool_call_id,
        "status": status,
        "content": params.get("content").cloned().unwrap_or(Value::Null),
        "structured_content": params.get("structured_content").cloned().unwrap_or(Value::Null),
        "error": params.get("error").cloned().unwrap_or(Value::Null),
    });
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
            let session = acp_sessions::find_for_user_bear_session(
                &state.sqlx_pool,
                user_id,
                &bear.slug,
                &session_id,
            )
            .await?
            .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
            let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, BearProfile::Pair)
                .await?
                .ok_or_else(|| CustomError::NotFound("Bear pair profile binding not found".to_string()))?;
            let content = params
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    params
                        .get("structured_content")
                        .or_else(|| params.get("error"))
                        .map(|v| v.to_string())
                        .unwrap_or_default()
                });
            let continuation_status = match status.as_str() {
                "ok" => RuntimeToolResultStatus::Ok,
                "timeout" | "timed_out" => RuntimeToolResultStatus::Timeout,
                _ => RuntimeToolResultStatus::Error,
            };
            spawn_continuation_task(
                state,
                transitioned.clone().unwrap_or(run.clone()),
                binding_id,
                session
                    .resolved_conversation_id
                    .clone()
                    .unwrap_or(session.conversation_id),
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
        "approved" | "approve" | "granted" | "allow" => "granted",
        "denied" | "deny" | "rejected" | "reject" => "denied",
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
            let session = acp_sessions::find_for_user_bear_session(
                &state.sqlx_pool,
                user_id,
                &bear.slug,
                &session_id,
            )
            .await?
            .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
            let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, BearProfile::Pair)
                .await?
                .ok_or_else(|| CustomError::NotFound("Bear pair profile binding not found".to_string()))?;
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
                session
                    .resolved_conversation_id
                    .clone()
                    .unwrap_or(session.conversation_id),
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
