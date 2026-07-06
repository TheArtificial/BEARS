use std::time::Duration;
use std::time::Instant;

use axum::http::HeaderMap;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use den_core::tools::{
    constants::DEN_WEB_FETCH,
    result_compaction::{
        compact_client_tool_result, compact_client_tool_result_with_artifact,
        ClientToolResultInput, ToolResultStatus,
    },
};
use den_http::{errors::CustomError, web_policy};
use den_protocol::{
    RoleRuntimeBinding, RuntimeApprovalDecision, RuntimeContinuation, RuntimeConversationRef,
    RuntimeToolResultStatus,
};
use den_runtime::{
    bearwire_events,
    client_obligation_coordinator::{
        self, PermissionResultCoordinatorOutcome, ToolResultCoordinatorOutcome,
    },
    native_runtime::continue_native_client_turn_event_stream,
    runtime::bearwire_projection::wire::{tool_call_finish_wire, BearWireEvent},
    tool_output_artifacts::{create_tool_output_artifact, ToolOutputArtifactInput},
    turn_obligations::{self, ExpectedResponderAction},
    turn_runner::{default_tool_continue_stream_context, TurnContinueRequest},
    turn_runs,
};
use den_service::{
    bears::{db as bears_db, BearProfile},
    client_sessions, DenState,
};

use crate::auth::authenticated_bear;
use crate::methods::run::{
    persist_run_failed, persist_run_progress, persist_runtime_event_as_bearwire,
};
use crate::methods::{
    deserialize_optional_string, deserialize_required_string, deserialize_string, parse_params,
};

fn deserialize_tool_result_status<'de, D>(deserializer: D) -> Result<ToolResultStatus, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = deserialize_string(deserializer)?;
    ToolResultStatus::parse(&raw)
        .ok_or_else(|| serde::de::Error::custom(format!("unsupported tool result status: {raw}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PermissionDecisionInput {
    Approved,
    Approve,
    Granted,
    Allow,
    AllowOnce,
    AllowSiteAccount,
    AllowHost,
    Denied,
    Deny,
    Rejected,
    Reject,
    RejectOnce,
    RejectAlways,
    Timeout,
    TimedOut,
}

impl PermissionDecisionInput {
    fn normalized(self) -> &'static str {
        match self {
            Self::Approved
            | Self::Approve
            | Self::Granted
            | Self::Allow
            | Self::AllowOnce
            | Self::AllowSiteAccount
            | Self::AllowHost => "granted",
            Self::Denied
            | Self::Deny
            | Self::Rejected
            | Self::Reject
            | Self::RejectOnce
            | Self::RejectAlways => "denied",
            Self::Timeout | Self::TimedOut => "expired",
        }
    }

    fn raw(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Approve => "approve",
            Self::Granted => "granted",
            Self::Allow => "allow",
            Self::AllowOnce => "allow_once",
            Self::AllowSiteAccount => "allow_site_account",
            Self::AllowHost => "allow_host",
            Self::Denied => "denied",
            Self::Deny => "deny",
            Self::Rejected => "rejected",
            Self::Reject => "reject",
            Self::RejectOnce => "reject_once",
            Self::RejectAlways => "reject_always",
            Self::Timeout => "timeout",
            Self::TimedOut => "timed_out",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ClientToolResultRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    run_id: String,
    #[serde(deserialize_with = "deserialize_required_string")]
    session_id: String,
    #[serde(deserialize_with = "deserialize_required_string")]
    tool_call_id: String,
    #[serde(
        default = "default_tool_result_status",
        deserialize_with = "deserialize_tool_result_status"
    )]
    status: ToolResultStatus,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    tool_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    content: Option<String>,
    /// Intentionally raw: tool-specific structured result payloads vary by tool family.
    #[serde(default)]
    structured_content: Value,
    /// Intentionally raw: some tools surface structured error objects instead of plain text.
    #[serde(default)]
    error: Value,
}

fn default_tool_result_status() -> ToolResultStatus {
    ToolResultStatus::Ok
}

impl ClientToolResultRequest {
    fn input(&self) -> ClientToolResultInput {
        ClientToolResultInput::new(
            self.tool_call_id.clone(),
            self.tool_name.clone(),
            self.status,
            self.content.clone(),
            self.structured_content.clone(),
            self.error.clone(),
        )
    }

    fn bearwire_finish_payload(&self, compacted: Value) -> Value {
        let error_message = (self.status != ToolResultStatus::Ok)
            .then(|| self.content.as_deref().or_else(|| self.error.as_str()))
            .flatten();
        serde_json::to_value(tool_call_finish_wire(
            &self.tool_call_id,
            self.tool_name.as_deref(),
            self.status.as_str(),
            None,
            error_message,
            self.content.as_deref(),
            (!self.structured_content.is_null()).then(|| self.structured_content.clone()),
            (!self.error.is_null()).then(|| self.error.clone()),
            Some(compacted),
        ))
        .expect("ToolCallFinishWire serializes")
    }
}

#[derive(Debug, Deserialize)]
struct ClientPermissionResultRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    run_id: String,
    #[serde(deserialize_with = "deserialize_required_string")]
    session_id: String,
    #[serde(deserialize_with = "deserialize_required_string")]
    permission_id: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    obligation_id: Option<String>,
    #[serde(default = "default_permission_decision")]
    decision: PermissionDecisionInput,
    /// Intentionally raw: reason may be string or structured adapter metadata.
    #[serde(default)]
    reason: Option<Value>,
}

fn default_permission_decision() -> PermissionDecisionInput {
    PermissionDecisionInput::Denied
}

fn continuation_watchdog_timeout() -> Duration {
    let millis = std::env::var("BEARS_BEARWIRE_CONTINUATION_WATCHDOG_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(1_000, 600_000))
        .unwrap_or(30_000);
    Duration::from_millis(millis)
}

fn continuation_conversation_id(session: &client_sessions::ClientSessionRow) -> String {
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
    if !matches!(decision, "allow_once" | "allow_site_account" | "allow_host") {
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
    } else if decision == "allow_site_account" {
        let scope_value = args
            .get("site_account")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                let normalized = web_policy::normalize_web_url(url).ok()?;
                if normalized.host == "github.com" {
                    let account = url
                        .split("github.com/")
                        .nth(1)?
                        .split('/')
                        .find(|segment| !segment.is_empty())?;
                    Some(format!("github.com/{account}"))
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                CustomError::ValidationError(
                    "web_fetch permission payload missing supported site account scope".to_string(),
                )
            })?;
        ("site_account", scope_value)
    } else {
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
    run: &turn_runs::TurnRunRow,
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
    run: turn_runs::TurnRunRow,
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
        let _ = turn_runs::transition_run(
            &pool,
            &run.run_id,
            turn_runs::TurnRunState::Continuing,
            None,
        )
        .await;
        let binding = RoleRuntimeBinding {
            binding_id,
            compatibility_backend: Some("native".to_string()),
        };
        let result = continue_native_client_turn_event_stream(
            TurnContinueRequest {
                sqlx_pool: &pool,
                config: config.as_ref(),
                memory_stores: &memory_stores,
                request_id,
                run_id: Some(&run.run_id),
                client_session_id: &run.session_id,
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
                let watchdog_timeout = continuation_watchdog_timeout();
                loop {
                    let item = match tokio::time::timeout(watchdog_timeout, stream.next()).await {
                        Ok(item) => item,
                        Err(_) => {
                            let context = json!({
                                "continuation_request_id": request_id,
                                "watchdog_timeout_ms": watchdog_timeout.as_millis(),
                                "runtime_event_count": runtime_event_count,
                                "first_event_seen": first_event_seen,
                                "terminal_event_seen": terminal_event_seen,
                                "last_event_kind": last_event_kind,
                            });
                            persist_run_failed(
                                &pool,
                                &run.session_id,
                                &run.run_id,
                                run.bear_id,
                                run.user_id,
                                "continuation_watchdog_timeout",
                                format!(
                                    "Den received the client result and started continuation request {request_id}, but no runtime event arrived within {}ms. This usually means the resumed model/runtime stream stalled before emitting its first event.",
                                    watchdog_timeout.as_millis()
                                ),
                                Some(context),
                            )
                            .await;
                            break;
                        }
                    };
                    let Some(item) = item else {
                        break;
                    };
                    match item {
                        Ok(runtime_event) => {
                            runtime_event_count += 1;
                            let event_kind =
                                crate::methods::run::runtime_event_kind(&runtime_event);
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
                                None,
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
                    None,
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
    let request: ClientToolResultRequest = parse_params(params)?;
    let run_id = request.run_id.clone();
    let session_id = request.session_id.clone();
    let tool_call_id = request.tool_call_id.clone();
    let status = request.status;
    let Some(run) = turn_runs::get_run(&state.sqlx_pool, &run_id).await? else {
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
        turn_obligations::get_tool_call_obligation(&state.sqlx_pool, &run_id, &tool_call_id)
            .await?
            .ok_or_else(|| {
                CustomError::ValidationError(
                    "BearWire tool result has no persisted tool-call obligation".to_string(),
                )
            })?;
    if !turn_obligations::obligation_accepts_responder_action(
        &obligation,
        ExpectedResponderAction::ToolResult,
    ) {
        return Err(CustomError::ValidationError(format!(
            "BearWire tool obligation {} does not accept client.tool.result (expected {}, state {})",
            obligation.id,
            obligation.expected_responder_action,
            obligation.state
        )));
    }
    let input = request.input();
    let mut compacted = compact_client_tool_result(&input);
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
                tool_name: request.tool_name.clone(),
                source: "bearwire_client",
                content_text: request.content.clone(),
                content_json: Some(params.clone()),
                metadata: json!({ "status": status.as_str() }),
            },
        )
        .await
        {
            compacted =
                compact_client_tool_result_with_artifact(&input, Some(&artifact.artifact_ref));
        }
    }
    let payload = compacted.payload.clone();
    let event_payload = request.bearwire_finish_payload(payload.clone());

    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let continuation_conversation_id = continuation_conversation_id(&session);
    if !den_runtime::native_runtime::native_client_session_exists(
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
    let coordinator_outcome = client_obligation_coordinator::record_and_settle_tool_result(
        &state.sqlx_pool,
        &run,
        &obligation,
        "tool",
        &tool_call_id,
        payload.clone(),
    )
    .await?;
    match coordinator_outcome {
        ToolResultCoordinatorOutcome::DuplicateConflict { existing_hash } => {
            Err(CustomError::ValidationError(format!(
                "conflicting duplicate tool result for {tool_call_id}; existing hash {existing_hash}"
            )))
        }
        ToolResultCoordinatorOutcome::DuplicateIdentical { result, run_state } => Ok(json!({
            "ok": true,
            "duplicate": true,
            "result_id": result.id,
            "run_state": run_state,
            "obligation_state": obligation.state,
        })),
        ToolResultCoordinatorOutcome::IgnoredLateResult {
            run_state,
            obligation_state,
        } => Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "run_state": run_state,
            "obligation_state": obligation_state,
        })),
        ToolResultCoordinatorOutcome::WaitingForMoreClientResults {
            run: transitioned,
            open_obligations,
            result,
        } => {
            let result = result.expect("record-and-settle tool outcome should include result row");
            let event_type = if status == ToolResultStatus::Ok {
                "tool_call.completed"
            } else {
                "tool_call.failed"
            };
            let mut event = BearWireEvent::ephemeral(event_type, event_payload.clone());
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
            let content = compacted.content.clone();
            let continuation_status = match status {
                ToolResultStatus::Ok => RuntimeToolResultStatus::Ok,
                ToolResultStatus::Timeout => RuntimeToolResultStatus::Timeout,
                ToolResultStatus::Error
                | ToolResultStatus::Incomplete
                | ToolResultStatus::Cancelled => RuntimeToolResultStatus::Error,
            };
            den_runtime::native_runtime::record_native_client_tool_result(
                &state.sqlx_pool,
                &continuation_conversation_id,
                &session_id,
                &Uuid::new_v4().to_string(),
                Some(&run_id),
                &tool_call_id,
                obligation.permission_id.as_deref(),
                continuation_status,
                content,
            )
            .await?;
            Ok(json!({
                "ok": true,
                "duplicate": false,
                "result_id": result.id,
                "event_sequence": persisted.sequence_no,
                "run_state": transitioned.map(|run| run.state).unwrap_or_else(|| "unknown".to_string()),
                "continuation": "waiting_for_more_client_results",
                "open_obligation_count": open_obligations.len(),
                "open_obligations": open_obligations.into_iter().map(|obligation| json!({
                    "obligation_id": obligation.id,
                    "kind": obligation.kind,
                    "expected_responder_action": obligation.expected_responder_action,
                    "tool_call_id": obligation.tool_call_id,
                    "permission_id": obligation.permission_id,
                    "state": obligation.state,
                })).collect::<Vec<_>>(),
            }))
        }
        ToolResultCoordinatorOutcome::ContinueModel {
            run: transitioned,
            result,
        } => {
            let result = result.expect("record-and-settle tool outcome should include result row");
            let event_type = if status == ToolResultStatus::Ok {
                "tool_call.completed"
            } else {
                "tool_call.failed"
            };
            let mut event = BearWireEvent::ephemeral(event_type, event_payload.clone());
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
            let content = compacted.content.clone();
            let continuation_status = match status {
                ToolResultStatus::Ok => RuntimeToolResultStatus::Ok,
                ToolResultStatus::Timeout => RuntimeToolResultStatus::Timeout,
                ToolResultStatus::Error
                | ToolResultStatus::Incomplete
                | ToolResultStatus::Cancelled => RuntimeToolResultStatus::Error,
            };
            spawn_continuation_task(
                state,
                transitioned.clone().unwrap_or_else(|| run.clone()),
                binding_id,
                continuation_conversation_id,
                RuntimeContinuation::ToolResult {
                    tool_call_id: tool_call_id.clone(),
                    approval_request_id: obligation.permission_id.clone(),
                    status: continuation_status,
                    content,
                },
            );
            Ok(json!({
                "ok": true,
                "duplicate": false,
                "result_id": result.id,
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
    let request: ClientPermissionResultRequest = parse_params(params)?;
    let run_id = request.run_id;
    let session_id = request.session_id;
    let permission_id = request.permission_id;
    let obligation_id = request.obligation_id;
    let decision = request.decision;
    let Some(run) = turn_runs::get_run(&state.sqlx_pool, &run_id).await? else {
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
        turn_obligations::get_permission_obligation(&state.sqlx_pool, &run_id, &permission_id)
            .await?
            .ok_or_else(|| {
                CustomError::ValidationError(
                    "BearWire permission result has no persisted permission obligation".to_string(),
                )
            })?;
    if let Some(obligation_id) = obligation_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if obligation.id.to_string() != obligation_id {
            return Err(CustomError::ValidationError(
                "BearWire permission result obligation_id does not match persisted permission obligation".to_string(),
            ));
        }
    }
    if !turn_obligations::obligation_accepts_responder_action(
        &obligation,
        ExpectedResponderAction::PermissionDecision,
    ) {
        return Err(CustomError::ValidationError(format!(
            "BearWire permission obligation {} does not accept client.permission.result (expected {}, state {})",
            obligation.id,
            obligation.expected_responder_action,
            obligation.state
        )));
    }
    let normalized_decision = decision.normalized();
    let payload = json!({
        "permission_id": permission_id,
        "decision": normalized_decision,
        "reason": request.reason.unwrap_or(Value::Null),
    });

    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let continuation_conversation_id = continuation_conversation_id(&session);
    if !den_runtime::native_runtime::native_client_session_exists(
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
    let coordinator_outcome = client_obligation_coordinator::record_and_settle_permission_result(
        &state.sqlx_pool,
        &run,
        &obligation,
        normalized_decision,
        "permission",
        &permission_id,
        payload.clone(),
    )
    .await?;
    match coordinator_outcome {
        PermissionResultCoordinatorOutcome::DuplicateConflict { existing_hash } => {
            Err(CustomError::ValidationError(format!(
                "conflicting duplicate permission result for {permission_id}; existing hash {existing_hash}"
            )))
        }
        PermissionResultCoordinatorOutcome::DuplicateIdentical { result, run_state } => Ok(json!({
            "ok": true,
            "duplicate": true,
            "result_id": result.id,
            "run_state": run_state,
            "obligation_state": obligation.state,
        })),
        PermissionResultCoordinatorOutcome::IgnoredLateResult {
            run_state,
            obligation_state,
        } => Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "run_state": run_state,
            "obligation_state": obligation_state,
        })),
        PermissionResultCoordinatorOutcome::DispatchLocalTool {
            run: transitioned,
            tool_obligation,
            tool_call_id,
            tool_name,
            args,
            result,
        } => {
            let result = result.expect("record-and-settle permission outcome should include result row");
            if normalized_decision == "granted" {
                record_web_fetch_approval_from_permission(
                    &state.sqlx_pool,
                    bear.id,
                    user_id,
                    decision.raw(),
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
            Ok(json!({
                "ok": true,
                "duplicate": false,
                "result_id": result.id,
                "event_sequence": persisted.sequence_no,
                "run_state": transitioned.map(|run| run.state).unwrap_or_else(|| "unknown".to_string()),
                "continuation": "waiting_for_tool_result",
                "obligation_state": tool_obligation.state,
                "local_tool_request": {
                    "tool_call_id": tool_call_id,
                    "tool_name": tool_name,
                    "result_tool_name": tool_name,
                    "args": args,
                    "permission_id": permission_id,
                    "obligation_id": obligation.id.to_string()
                }
            }))
        }
        PermissionResultCoordinatorOutcome::ContinueModel { run: transitioned, result } => {
            let result = result.expect("record-and-settle permission outcome should include result row");
            if normalized_decision == "granted" {
                record_web_fetch_approval_from_permission(
                    &state.sqlx_pool,
                    bear.id,
                    user_id,
                    decision.raw(),
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
                transitioned.clone().unwrap_or_else(|| run.clone()),
                binding_id,
                continuation_conversation_id,
                RuntimeContinuation::ApprovalDecision {
                    approval_request_id: permission_id.clone(),
                    tool_call_id: obligation.tool_call_id.clone(),
                    decision,
                    reason,
                },
            );
            Ok(json!({
                "ok": true,
                "duplicate": false,
                "result_id": result.id,
                "event_sequence": persisted.sequence_no,
                "run_state": transitioned.map(|run| run.state).unwrap_or_else(|| "unknown".to_string()),
                "continuation": "started",
            }))
        }
    }
}
