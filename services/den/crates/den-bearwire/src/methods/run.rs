use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use futures::StreamExt;
use serde_json::{json, Value};
use uuid::Uuid;

use den_http::errors::CustomError;
use den_protocol::RoleRuntimeBinding;
use den_runtime::{
    bearwire_events, bearwire_obligations, bearwire_run_steps, bearwire_runs,
    native_runtime::start_native_client_turn_event_stream,
    runtime::bearwire_projection::wire::{runtime_stream_event_to_bearwire_events, BearWireEvent},
    turn_runner::TurnStartRequest,
};
use den_service::{
    bears::{db as bears_db, BearProfile},
    bifrost::BifrostCatalogSnapshot,
    client_sessions, DenState,
};

use crate::auth::authenticated_bear;
use crate::methods::{param_string, required_param_string};

const BEARWIRE_EAGER_PREFIX_DRIVE_TIMEOUT: Duration = Duration::from_millis(3_000);
const BEARWIRE_LOG_SAMPLE_CHARS: usize = 2_000;

fn log_sample(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    let mut out = value
        .chars()
        .take(BEARWIRE_LOG_SAMPLE_CHARS)
        .collect::<String>();
    if value.chars().count() > BEARWIRE_LOG_SAMPLE_CHARS {
        out.push_str("…");
    }
    out
}

fn client_tool_descriptors_from_context(
    client_context: Option<&Value>,
    requested_mode: Option<&str>,
) -> Value {
    let context = client_context.unwrap_or(&Value::Null);
    let policy = den_core::client_tools::resolve_session_policy_for_mode(
        requested_mode.unwrap_or("ask"),
        None,
    );
    let mut descriptors = Vec::new();
    for tool in den_core::client_tools::ClientToolName::all() {
        if *tool == den_core::client_tools::ClientToolName::McpCallTool
            || !policy.allows_tool(*tool)
        {
            continue;
        }
        let descriptor = tool.descriptor();
        if !adapter_supports_tool(context, descriptor.provider_name) {
            continue;
        }
        descriptors.push(den_core::client_tools::provider_tool_descriptor(*tool));
    }
    if let Some(mcp_tools) = context
        .pointer("/mcp/client_tools")
        .and_then(Value::as_array)
    {
        descriptors.extend(mcp_tools.iter().cloned());
    }
    if descriptors.is_empty() {
        descriptors.push(den_core::client_tools::provider_tool_descriptor(
            den_core::client_tools::ClientToolName::ReadTextFile,
        ));
    }
    json!(descriptors)
}

fn runtime_upstream_target(
    conversation_selection: &str,
    resolved_conversation_id: Option<&str>,
) -> String {
    resolved_conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(conversation_selection)
        .to_string()
}

#[derive(Debug, Clone)]
struct ResolvedRunModel {
    handle: String,
    provider_model_id: String,
    /// Set authoritatively by `preflight_pair_run_model` from the catalog
    /// snapshot; `resolve_pair_run_model` leaves it at a placeholder.
    api_style: den_llm::LlmApiStyle,
    source: &'static str,
}

/// Placeholder used while resolving model identity; overwritten by preflight.
const RESOLVE_PLACEHOLDER_API_STYLE: den_llm::LlmApiStyle =
    den_llm::LlmApiStyle::ChatCompletionsStream;

fn provider_model_id_for_den_handle(handle: &str) -> String {
    den_llm::model_registry::provider_model_id_for_handle(handle)
        .unwrap_or(handle.trim())
        .to_string()
}

fn available_model_matches(
    model: &den_service::bifrost::BifrostModelMetadata,
    resolved: &ResolvedRunModel,
) -> bool {
    model.handle == resolved.handle
        || model.handle == resolved.provider_model_id
        || model.model == resolved.handle
        || model.model == resolved.provider_model_id
}

fn available_model_sample(models: &[den_service::bifrost::BifrostModelMetadata]) -> String {
    let mut handles = models
        .iter()
        .map(|model| model.handle.as_str())
        .take(20)
        .collect::<Vec<_>>();
    handles.sort_unstable();
    handles.join(", ")
}

async fn resolve_pair_run_model(
    state: &DenState,
    bear: &den_service::bears::Bear,
    conversation_id: &str,
) -> Result<ResolvedRunModel, CustomError> {
    if let Some(conversation) =
        den_service::conversation::persistence::get_conversation_for_external_id(
            &state.sqlx_pool,
            bear.id,
            conversation_id,
        )
        .await?
    {
        if let Some(model_state) =
            den_service::conversation::persistence::get_conversation_model_state(
                &state.sqlx_pool,
                conversation.id,
            )
            .await?
        {
            if model_state.selection_mode == "explicit" {
                if let Some(model) = model_state
                    .selected_model
                    .or(model_state.requested_model)
                    .map(|model| model.trim().to_string())
                    .filter(|model| !model.is_empty())
                {
                    let handle = den_llm::normalize_llm_model_handle(&model);
                    let provider_model_id = provider_model_id_for_den_handle(&handle);
                    return Ok(ResolvedRunModel {
                        api_style: RESOLVE_PLACEHOLDER_API_STYLE,
                        provider_model_id,
                        handle,
                        source: "conversation_explicit",
                    });
                }
            } else if let Some(model) = model_state
                .selected_model
                .map(|model| model.trim().to_string())
                .filter(|model| !model.is_empty())
            {
                let handle = den_llm::normalize_llm_model_handle(&model);
                let provider_model_id = provider_model_id_for_den_handle(&handle);
                return Ok(ResolvedRunModel {
                    api_style: RESOLVE_PLACEHOLDER_API_STYLE,
                    provider_model_id,
                    handle,
                    source: "conversation_auto",
                });
            }
        }
    }

    if let Some(model) =
        bears_db::profile_model_setting(&state.sqlx_pool, bear.id, BearProfile::Pair).await?
    {
        let handle = den_llm::normalize_llm_model_handle(&model);
        let provider_model_id = provider_model_id_for_den_handle(&handle);
        return Ok(ResolvedRunModel {
            api_style: RESOLVE_PLACEHOLDER_API_STYLE,
            provider_model_id,
            handle,
            source: "profile_default",
        });
    }

    if let Some(model) = bear
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        let handle = den_llm::normalize_llm_model_handle(model);
        let provider_model_id = provider_model_id_for_den_handle(&handle);
        return Ok(ResolvedRunModel {
            api_style: RESOLVE_PLACEHOLDER_API_STYLE,
            provider_model_id,
            handle,
            source: "bear_default",
        });
    }

    let handle = den_llm::normalize_llm_model_handle(&state.config.default_llm_model);
    let provider_model_id = provider_model_id_for_den_handle(&handle);
    Ok(ResolvedRunModel {
        api_style: RESOLVE_PLACEHOLDER_API_STYLE,
        provider_model_id,
        handle,
        source: "system_default",
    })
}

async fn preflight_pair_run_model(
    state: &DenState,
    bear: &den_service::bears::Bear,
    session_id: &str,
    conversation_id: &str,
) -> Result<ResolvedRunModel, CustomError> {
    let resolved = resolve_pair_run_model(state, bear, conversation_id).await?;
    let snapshot = match state
        .bifrost
        .bear_catalog_snapshot(
            &state.sqlx_pool,
            bear.id,
            &state.config.den_secret_encryption_key,
        )
        .await
    {
        Ok(snapshot) => snapshot,
        Err(err) => {
            tracing::warn!(
                error = %err,
                bear_id = %bear.id,
                "Bear-scoped Bifrost catalog refresh before Pair preflight failed; proceeding without catalog validation"
            );
            BifrostCatalogSnapshot::default()
        }
    };
    let available = snapshot.models_vec();
    let catalog_entry = snapshot.resolve(&resolved.handle);
    let resolved = ResolvedRunModel {
        api_style: den_llm::preferred_api_style_for_model_with_catalog_support(
            &resolved.handle,
            catalog_entry.and_then(|entry| entry.supports_responses_api),
        ),
        ..resolved
    };
    if available
        .iter()
        .any(|model| available_model_matches(model, &resolved))
    {
        tracing::info!(
            session_id,
            bear_id = %bear.id,
            conversation_id,
            model_handle = %resolved.handle,
            provider_model_id = %resolved.provider_model_id,
            model_selection_source = resolved.source,
            api_style = %resolved.api_style.as_str(),
            catalog_stale = snapshot.stale,
            catalog_fetched_at = ?snapshot.fetched_at,
            "BearWire model preflight passed"
        );
        return Ok(resolved);
    }

    tracing::warn!(
        session_id,
        bear_id = %bear.id,
        conversation_id,
        model_handle = %resolved.handle,
        provider_model_id = %resolved.provider_model_id,
        model_selection_source = resolved.source,
        api_style = %resolved.api_style.as_str(),
        available_models = %available_model_sample(&available),
        catalog_stale = snapshot.stale,
        catalog_fetched_at = ?snapshot.fetched_at,
        "BearWire model preflight did not find selected model in catalog snapshot; proceeding and letting Bifrost execution validate"
    );
    Ok(resolved)
}

fn adapter_supports_tool(client_context: &Value, provider_name: &str) -> bool {
    client_context
        .pointer(&format!("/adapter/direct_tools/{provider_name}/supported"))
        .and_then(Value::as_bool)
        .or_else(|| {
            client_context
                .pointer(&format!("/direct_tools/{provider_name}"))
                .and_then(Value::as_bool)
        })
        .unwrap_or(false)
}

pub(crate) async fn persist_run_progress(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    started_at: Instant,
    kind: &str,
    text: &str,
    detail: Value,
) {
    tracing::info!(
        session_id = %session_id,
        run_id = %run_id,
        kind,
        elapsed_ms = started_at.elapsed().as_millis(),
        detail = %detail,
        "BearWire run progress"
    );
    let mut event = BearWireEvent::ephemeral(
        "run.progress",
        json!({
            "kind": kind,
            "text": text,
            "elapsed_ms": started_at.elapsed().as_millis(),
            "detail": detail,
        }),
    );
    event.bear_id = Some(bear_id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.to_string());
    event.run_id = Some(run_id.to_string());
    if let Err(err) = bearwire_events::append_bearwire_event(
        pool,
        session_id,
        Some(bear_id),
        Some(user_id),
        event,
    )
    .await
    {
        tracing::warn!(
            error = %err,
            session_id = %session_id,
            run_id = %run_id,
            kind,
            "failed to persist BearWire run.progress event"
        );
    }
}

fn runtime_event_satisfies_eager_prefix(event: &den_protocol::RuntimeStreamEvent) -> bool {
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
    matches!(
        event,
        RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::AssistantTextDelta { .. }
                | RuntimeSemanticEvent::ToolCallRequested { .. }
                | RuntimeSemanticEvent::RunPaused { .. }
                | RuntimeSemanticEvent::TurnCompleted { .. }
                | RuntimeSemanticEvent::TurnFailed { .. }
                | RuntimeSemanticEvent::TurnCancelled { .. }
                | RuntimeSemanticEvent::Error { .. }
        )
    )
}

pub(crate) fn runtime_event_kind(event: &den_protocol::RuntimeStreamEvent) -> &'static str {
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
    match event {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { .. }) => {
            "assistant_text_delta"
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::StatusText { .. }) => "status_text",
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress { .. }) => "run_progress",
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunPaused { .. }) => "run_paused",
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested { .. }) => {
            "tool_call_requested"
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallFinished { .. }) => {
            "tool_call_finished"
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ConversationResolved { .. }) => {
            "conversation_resolved"
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. }) => {
            "turn_completed"
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { .. }) => "turn_failed",
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCancelled { .. }) => {
            "turn_cancelled"
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error { .. }) => "error",
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => "untranslated_provider_event",
    }
}

pub(crate) async fn persist_runtime_event_as_bearwire(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    runtime_event: den_protocol::RuntimeStreamEvent,
    request_id: Uuid,
    started_at: Option<Instant>,
) {
    let active_obligation = update_run_state_for_runtime_event(
        pool,
        session_id,
        run_id,
        bear_id,
        user_id,
        &runtime_event,
        request_id,
        started_at,
    )
    .await;
    for mut event in runtime_stream_event_to_bearwire_events(runtime_event) {
        if event.event_type == "client.waiting" {
            if let Some(obligation) = active_obligation.as_ref() {
                event.data["obligation_id"] = json!(obligation.id.to_string());
                event.data["expected_client_method"] = json!(obligation.expected_client_method);
                event.data["permission_id"] = json!(obligation.permission_id);
                event.data["tool_call_id"] = json!(obligation.tool_call_id);
                event.resource_refs.push(
                    den_runtime::runtime::bearwire_projection::wire::ResourceRef::new(
                        "client_obligation",
                        obligation.id.to_string(),
                    ),
                );
            } else {
                tracing::warn!(
                    session_id = %session_id,
                    run_id = %run_id,
                    "runtime emitted client.waiting event without persisted BearWire obligation; dropping unanswerable event"
                );
                continue;
            }
        }
        if event.event_type == "permission.requested" {
            if active_obligation.is_none() {
                tracing::warn!(
                    session_id = %session_id,
                    run_id = %run_id,
                    "runtime emitted permission.requested event without persisted BearWire obligation; dropping unanswerable event"
                );
                continue;
            }
        }
        event.bear_id = Some(bear_id.to_string());
        event.human_id = Some(user_id.to_string());
        event.session_id = Some(session_id.to_string());
        if event.run_id.is_none() {
            event.run_id = Some(run_id.to_string());
        }
        if let Err(err) = bearwire_events::append_bearwire_event(
            pool,
            session_id,
            Some(bear_id),
            Some(user_id),
            event,
        )
        .await
        {
            tracing::warn!(error = %err, session_id = %session_id, "failed to persist BearWire runtime event");
        }
    }
}

pub(crate) async fn persist_run_failed(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    reason: &str,
    message: String,
) {
    tracing::warn!(
        session_id,
        run_id,
        bear_id = %bear_id,
        user_id,
        reason,
        error_message = %log_sample(&message),
        "BearWire run failed"
    );
    let _ = bearwire_runs::transition_run(
        pool,
        run_id,
        bearwire_runs::BearWireRunState::Failed,
        Some(reason),
    )
    .await;
    let _ = bearwire_obligations::settle_outstanding_for_run(
        pool,
        run_id,
        bearwire_obligations::BearWireObligationState::Failed,
    )
    .await;
    let mut event = BearWireEvent::ephemeral(
        "run.failed",
        json!({
            "run_id": run_id,
            "message": message,
            "reason": reason,
        }),
    );
    event.bear_id = Some(bear_id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.to_string());
    event.run_id = Some(run_id.to_string());
    let _ = bearwire_events::append_bearwire_event(
        pool,
        session_id,
        Some(bear_id),
        Some(user_id),
        event,
    )
    .await;
}

async fn update_run_state_for_runtime_event(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    event: &den_protocol::RuntimeStreamEvent,
    request_id: Uuid,
    started_at: Option<Instant>,
) -> Option<bearwire_obligations::BearWireRunObligationRow> {
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
    match event {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            arguments,
            approval_request_id,
            approval_required,
            ..
        }) => {
            let has_permission_id = approval_request_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|id| !id.is_empty());
            if *approval_required && !has_permission_id {
                tracing::warn!(
                    session_id,
                    run_id,
                    tool_call_id,
                    tool_name,
                    "runtime emitted approval_required tool call without approval_request_id; treating as tool result obligation"
                );
            }
            let effective_approval_required = *approval_required && has_permission_id;
            let state = if effective_approval_required {
                bearwire_runs::BearWireRunState::WaitingForPermission
            } else {
                bearwire_runs::BearWireRunState::WaitingForToolResult
            };
            let _ = bearwire_runs::transition_run(pool, run_id, state, None).await;
            let step_id = match bearwire_run_steps::ensure_active_step(pool, run_id).await {
                Ok(step) => Some(step.id),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        session_id = %session_id,
                        run_id = %run_id,
                        tool_call_id = %tool_call_id,
                        "failed to ensure BearWire run step for tool-call obligation"
                    );
                    None
                }
            };
            let request_payload = json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "arguments": arguments,
                "approval_required": effective_approval_required,
                "approval_request_id": approval_request_id,
                "request_id": request_id,
            });
            let obligation = if effective_approval_required {
                if let Some(permission_id) = approval_request_id.as_deref() {
                    bearwire_obligations::upsert_permission_obligation_for_step(
                        pool,
                        run_id,
                        session_id,
                        step_id,
                        permission_id,
                        Some(tool_call_id),
                        request_payload,
                    )
                    .await
                } else {
                    bearwire_obligations::upsert_tool_call_obligation_for_step(
                        pool,
                        run_id,
                        session_id,
                        step_id,
                        tool_call_id,
                        None,
                        request_payload,
                    )
                    .await
                }
            } else {
                bearwire_obligations::upsert_tool_call_obligation_for_step(
                    pool,
                    run_id,
                    session_id,
                    step_id,
                    tool_call_id,
                    approval_request_id.as_deref(),
                    request_payload,
                )
                .await
            };
            let obligation = match obligation {
                Ok(obligation) => Some(obligation),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        session_id = %session_id,
                        run_id = %run_id,
                        tool_call_id = %tool_call_id,
                        "failed to persist BearWire tool-call obligation"
                    );
                    None
                }
            };
            if let Some(started_at) = started_at {
                let (kind, text) = if effective_approval_required {
                    (
                        "tool_waiting_for_permission",
                        "Waiting for client permission to run a local tool…",
                    )
                } else {
                    (
                        "tool_waiting_for_result",
                        "Waiting for local tool result from the armature…",
                    )
                };
                persist_run_progress(
                    pool,
                    session_id,
                    run_id,
                    bear_id,
                    user_id,
                    started_at,
                    kind,
                    text,
                    json!({
                        "tool_call_id": tool_call_id,
                        "tool_name": tool_name,
                        "arguments": arguments,
                        "approval_required": effective_approval_required,
                        "approval_request_id": approval_request_id,
                        "request_id": request_id,
                    }),
                )
                .await;
            }
            obligation
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. }) => {
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                bearwire_runs::BearWireRunState::Completed,
                Some("completed"),
            )
            .await;
            let _ = bearwire_obligations::settle_outstanding_for_run(
                pool,
                run_id,
                bearwire_obligations::BearWireObligationState::Continued,
            )
            .await;
            let _ = bearwire_run_steps::transition_active_steps_for_run(pool, run_id, "continued")
                .await;
            None
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed {
            category,
            message,
            ..
        }) => {
            let reason = format!("{:?}", category);
            tracing::warn!(
                session_id,
                run_id,
                bear_id = %bear_id,
                user_id,
                %request_id,
                elapsed_ms = started_at.map(|started| started.elapsed().as_millis()),
                reason = %reason,
                error_message = %log_sample(message),
                "BearWire runtime turn failed"
            );
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                bearwire_runs::BearWireRunState::Failed,
                Some(&reason),
            )
            .await;
            let _ = bearwire_obligations::settle_outstanding_for_run(
                pool,
                run_id,
                bearwire_obligations::BearWireObligationState::Failed,
            )
            .await;
            let _ =
                bearwire_run_steps::transition_active_steps_for_run(pool, run_id, "failed").await;
            None
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCancelled { .. }) => {
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                bearwire_runs::BearWireRunState::Cancelled,
                Some("cancelled"),
            )
            .await;
            let _ = bearwire_obligations::settle_outstanding_for_run(
                pool,
                run_id,
                bearwire_obligations::BearWireObligationState::Cancelled,
            )
            .await;
            let _ = bearwire_run_steps::transition_active_steps_for_run(pool, run_id, "cancelled")
                .await;
            None
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error {
            message,
            detail,
            error_type,
            request_id: event_request_id,
            context,
        }) => {
            tracing::warn!(
                session_id,
                run_id,
                bear_id = %bear_id,
                user_id,
                %request_id,
                event_request_id = event_request_id.as_deref(),
                elapsed_ms = started_at.map(|started| started.elapsed().as_millis()),
                error_type = error_type.as_deref(),
                error_message = %log_sample(message),
                detail = detail.as_deref().map(log_sample),
                context = context.as_ref().map(|value| log_sample(value.to_string())),
                "BearWire runtime emitted error event"
            );
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                bearwire_runs::BearWireRunState::Failed,
                error_type.as_deref().or(Some("error")),
            )
            .await;
            let _ = bearwire_obligations::settle_outstanding_for_run(
                pool,
                run_id,
                bearwire_obligations::BearWireObligationState::Failed,
            )
            .await;
            let _ =
                bearwire_run_steps::transition_active_steps_for_run(pool, run_id, "failed").await;
            None
        }
        _ => None,
    }
}

pub(crate) async fn run_start_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let prompt = required_param_string(params, "prompt")?;
    let prompt_context = params.get("prompt_context").cloned();
    let client = param_string(params, "client").unwrap_or_else(|| "bearwire".to_string());
    let existing = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?;
    let conversation_id = param_string(params, "conversation_id")
        .or_else(|| {
            existing
                .as_ref()
                .map(|session| session.conversation_id.clone())
        })
        .unwrap_or_else(|| format!("new-acp-{client}-{}", Uuid::new_v4().simple()));
    let resolved_conversation_id = existing
        .as_ref()
        .and_then(|session| session.resolved_conversation_id.clone());
    let upstream_target =
        runtime_upstream_target(&conversation_id, resolved_conversation_id.as_deref());
    let cwd = param_string(params, "cwd");
    let requested_mode =
        param_string(params, "requested_mode").or_else(|| param_string(params, "mode"));
    let client_tools = client_tool_descriptors_from_context(
        params.get("client_context"),
        requested_mode.as_deref(),
    );
    let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, BearProfile::Pair)
        .await?
        .ok_or_else(|| CustomError::NotFound("Bear pair profile binding not found".to_string()))?;
    let binding = RoleRuntimeBinding {
        binding_id,
        compatibility_backend: Some("native".to_string()),
    };
    let resolved_model =
        preflight_pair_run_model(&state, &bear, &session_id, &upstream_target).await?;
    client_sessions::upsert_session(
        &state.sqlx_pool,
        client_sessions::UpsertClientSession {
            user_id,
            bear_id: bear.id,
            bear_slug: bear.slug.clone(),
            client_session_id: session_id.clone(),
            runtime_session_id: existing
                .as_ref()
                .map(|session| session.runtime_session_id.clone())
                .unwrap_or_else(|| format!("bearwire:{}:{}", bear.id, session_id)),
            conversation_id: conversation_id.clone(),
            resolved_conversation_id: resolved_conversation_id.clone(),
            client: client.clone(),
            cwd: cwd.clone(),
            current_mode: None,
        },
    )
    .await?;

    if let Some(active_run) = bearwire_runs::supersede_active_run_for_session(
        &state.sqlx_pool,
        &session_id,
        bear.id,
        user_id,
        "superseded_by_new_run",
    )
    .await?
    {
        let _ = bearwire_obligations::settle_outstanding_for_run(
            &state.sqlx_pool,
            &active_run.run_id,
            bearwire_obligations::BearWireObligationState::Failed,
        )
        .await;
    }

    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let run =
        bearwire_runs::create_run(&state.sqlx_pool, &run_id, &session_id, bear.id, user_id).await?;
    let mut accepted = BearWireEvent::ephemeral(
        "run.accepted",
        json!({
            "run_id": run_id.clone(),
            "session_id": session_id.clone(),
        }),
    );
    accepted.bear_id = Some(bear.id.to_string());
    accepted.human_id = Some(user_id.to_string());
    accepted.session_id = Some(session_id.clone());
    accepted.run_id = Some(run_id.clone());
    let accepted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        accepted,
    )
    .await?;

    let pool = state.sqlx_pool.clone();
    let config = state.config.clone();
    let memory_stores = state.memory_stores.clone();
    let bear_slug = bear.slug.clone();
    let bear_id = bear.id;
    let session_for_task = session_id.clone();
    let conversation_for_task = conversation_id.clone();
    let upstream_target_for_task = upstream_target.clone();
    let prompt_for_task = prompt.clone();
    let run_id_for_task = run_id.clone();
    let client_tools_for_task = client_tools.clone();
    let api_style_for_task = resolved_model.api_style;
    let (eager_prefix_tx, eager_prefix_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let mut eager_prefix_tx = Some(eager_prefix_tx);
        let run_started_at = Instant::now();
        let request_id = Uuid::new_v4();
        persist_run_progress(
            &pool,
            &session_for_task,
            &run_id_for_task,
            bear_id,
            user_id,
            run_started_at,
            "run_background_started",
            "Starting Pair stance run…",
            json!({
                "request_id": request_id,
                "conversation_id": conversation_for_task.clone(),
                "upstream_target": upstream_target_for_task.clone(),
                "client": client.clone(),
                "cwd": cwd.clone(),
                "client_tool_count": client_tools_for_task.as_array().map(|items| items.len()).unwrap_or(0),
            }),
        )
        .await;
        let _ = bearwire_runs::transition_run(
            &pool,
            &run_id_for_task,
            bearwire_runs::BearWireRunState::Running,
            None,
        )
        .await;
        let mut started = BearWireEvent::ephemeral(
            "run.started",
            json!({
                "run_id": run_id_for_task.clone(),
                "session_id": session_for_task.clone(),
            }),
        );
        started.bear_id = Some(bear_id.to_string());
        started.human_id = Some(user_id.to_string());
        started.session_id = Some(session_for_task.clone());
        started.run_id = Some(run_id_for_task.clone());
        let _ = bearwire_events::append_bearwire_event(
            &pool,
            &session_for_task,
            Some(bear_id),
            Some(user_id),
            started,
        )
        .await;
        persist_run_progress(
            &pool,
            &session_for_task,
            &run_id_for_task,
            bear_id,
            user_id,
            run_started_at,
            "native_context_assembling",
            "Preparing Pair stance context and tool surface…",
            json!({
                "request_id": request_id,
                "client_tool_count": client_tools_for_task.as_array().map(|items| items.len()).unwrap_or(0),
                "prompt_chars": prompt_for_task.chars().count(),
            }),
        )
        .await;
        let native_start = Instant::now();
        let stream_result = start_native_client_turn_event_stream(TurnStartRequest {
            sqlx_pool: &pool,
            config: config.as_ref(),
            memory_stores: &memory_stores,
            request_id,
            run_id: Some(&run_id_for_task),
            user_id,
            session_id: &session_for_task,
            bear_id,
            bear_slug: &bear_slug,
            client: &client,
            cwd: cwd.as_deref(),
            binding: &binding,
            conversation_selection: &conversation_for_task,
            upstream_target: &upstream_target_for_task,
            prompt: &prompt_for_task,
            prompt_context: prompt_context.clone(),
            client_tools: Some(client_tools_for_task),
            runtime_context: None,
            runtime_context_len: 0,
            stream_tokens: true,
            api_style: Some(api_style_for_task),
        })
        .await;

        match stream_result {
            Ok(mut stream) => {
                persist_run_progress(
                    &pool,
                    &session_for_task,
                    &run_id_for_task,
                    bear_id,
                    user_id,
                    run_started_at,
                    "model_stream_waiting",
                    "Context is ready; waiting for model output or tool request…",
                    json!({
                        "request_id": request_id,
                        "native_context_ms": native_start.elapsed().as_millis(),
                    }),
                )
                .await;
                let mut first_event_seen = false;
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(runtime_event) => {
                            if !first_event_seen
                                && runtime_event_satisfies_eager_prefix(&runtime_event)
                            {
                                if let Some(tx) = eager_prefix_tx.take() {
                                    let _ = tx.send(());
                                }
                            }
                            if !first_event_seen {
                                first_event_seen = true;
                                persist_run_progress(
                                    &pool,
                                    &session_for_task,
                                    &run_id_for_task,
                                    bear_id,
                                    user_id,
                                    run_started_at,
                                    "first_runtime_event",
                                    "Received first runtime event from model/native loop.",
                                    json!({
                                        "request_id": request_id,
                                        "event_kind": runtime_event_kind(&runtime_event),
                                    }),
                                )
                                .await;
                            }
                            persist_runtime_event_as_bearwire(
                                &pool,
                                &session_for_task,
                                &run_id_for_task,
                                bear_id,
                                user_id,
                                runtime_event,
                                request_id,
                                Some(run_started_at),
                            )
                            .await;
                        }
                        Err(err) => {
                            if let Some(tx) = eager_prefix_tx.take() {
                                let _ = tx.send(());
                            }
                            persist_run_failed(
                                &pool,
                                &session_for_task,
                                &run_id_for_task,
                                bear_id,
                                user_id,
                                "stream_error",
                                err.to_string(),
                            )
                            .await;
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                if let Some(tx) = eager_prefix_tx.take() {
                    let _ = tx.send(());
                }
                persist_run_failed(
                    &pool,
                    &session_for_task,
                    &run_id_for_task,
                    bear_id,
                    user_id,
                    "start_failed",
                    err.to_string(),
                )
                .await;
            }
        }
    });

    if tokio::time::timeout(BEARWIRE_EAGER_PREFIX_DRIVE_TIMEOUT, eager_prefix_rx)
        .await
        .is_err()
    {
        tracing::info!(
            session_id = %session_id,
            run_id = %run_id,
            timeout_ms = BEARWIRE_EAGER_PREFIX_DRIVE_TIMEOUT.as_millis(),
            "BearWire eager prefix drive timed out before first semantic runtime event"
        );
    }

    Ok(json!({
        "ok": true,
        "accepted": true,
        "run_id": run_id,
        "session_id": session_id,
        "event_sequence": accepted.sequence_no,
        "state": run.state,
    }))
}

pub(crate) async fn run_cancel_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let Some(session) = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    else {
        return Ok(json!({
            "ok": true,
            "cancelled": false,
            "session_id": session_id,
            "reason": "session_not_found",
        }));
    };

    let stream_cancel = state
        .turn_cancellations
        .cancel_session(&session.client_session_id);
    let active_turn = state
        .tool_turns
        .cancel_active_turn(&session.client_session_id);
    let active_run = bearwire_runs::active_run_for_session(&state.sqlx_pool, &session_id).await?;
    if let Some(run) = &active_run {
        let _ = bearwire_runs::transition_run(
            &state.sqlx_pool,
            &run.run_id,
            bearwire_runs::BearWireRunState::Cancelled,
            Some("client_requested"),
        )
        .await?;
        let _ = bearwire_obligations::settle_outstanding_for_run(
            &state.sqlx_pool,
            &run.run_id,
            bearwire_obligations::BearWireObligationState::Cancelled,
        )
        .await?;
    }
    let cancelled = stream_cancel.is_some() || active_turn.is_some() || active_run.is_some();
    let run_ids = stream_cancel
        .as_ref()
        .map(|turn| turn.run_ids.clone())
        .unwrap_or_default();

    let mut event = BearWireEvent::ephemeral(
        "run.cancelled",
        json!({
            "session_id": session_id,
            "cancelled": cancelled,
            "run_ids": run_ids,
            "run_id": active_run.as_ref().map(|run| run.run_id.clone()),
            "reason": if cancelled { "client_requested" } else { "no_active_run" },
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
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
        "cancelled": cancelled,
        "session_id": session_id,
        "run_ids": run_ids,
        "run_id": active_run.as_ref().map(|run| run.run_id.clone()),
        "active_turn": active_turn.map(|turn| turn.diagnostic()),
        "event_sequence": persisted.sequence_no,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor_names(value: &Value) -> Vec<&str> {
        value
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.get("name").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn runtime_upstream_target_prefers_resolved_conversation_for_history() {
        assert_eq!(
            runtime_upstream_target("new-acp-zed-pending", Some("den-conv-existing")),
            "den-conv-existing"
        );
    }

    #[test]
    fn runtime_upstream_target_falls_back_to_session_selection() {
        assert_eq!(
            runtime_upstream_target("new-acp-zed-pending", None),
            "new-acp-zed-pending"
        );
        assert_eq!(
            runtime_upstream_target("conv-existing", Some("   ")),
            "conv-existing"
        );
    }

    fn available_model(handle: &str, model: &str) -> den_service::bifrost::BifrostModelMetadata {
        den_service::bifrost::BifrostModelMetadata {
            handle: handle.to_string(),
            provider: "openai".to_string(),
            model: model.to_string(),
            display_name: None,
            context_window: 0,
            max_output_tokens: None,
            enabled: true,
            supports_tools: None,
            supports_responses_api: None,
            supports_vision: None,
        }
    }

    #[test]
    fn available_model_matches_den_handle_or_provider_model_id() {
        let resolved = ResolvedRunModel {
            handle: "openai/gpt-5.5".to_string(),
            provider_model_id: "gpt-5.5".to_string(),
            api_style: den_llm::LlmApiStyle::ResponsesStream,
            source: "conversation_explicit",
        };

        assert!(available_model_matches(
            &available_model("openai/gpt-5.5", "gpt-5.5"),
            &resolved
        ));
        assert!(available_model_matches(
            &available_model("gpt-5.5", "gpt-5.5"),
            &resolved
        ));
        assert!(!available_model_matches(
            &available_model("openai/gpt-5.1", "gpt-5.1"),
            &resolved
        ));
    }

    #[test]
    fn bearwire_advertises_supported_direct_and_forwarded_mcp_tools() {
        let context = json!({
            "adapter": {
                "direct_tools": {
                    "fs_read_text_file": { "supported": true },
                    "fs_find_paths": { "supported": true },
                    "fs_edit_file": { "supported": true },
                    "terminal_run_command": { "supported": true },
                    "chrome_open": { "supported": true }
                }
            },
            "mcp": {
                "client_tools": [{
                    "name": "mcp__chrome_devtools_custom__click",
                    "description": "Click",
                    "parameters": { "type": "object", "properties": { "uid": { "type": "string" } }, "required": ["uid"] }
                }]
            }
        });
        let descriptors = client_tool_descriptors_from_context(Some(&context), Some("write"));
        let names = descriptor_names(&descriptors);
        assert!(names.contains(&"fs_read_text_file"));
        assert!(names.contains(&"fs_find_paths"));
        assert!(names.contains(&"fs_edit_file"));
        assert!(names.contains(&"terminal_run_command"));
        assert!(names.contains(&"chrome_open"));
        assert!(names.contains(&"mcp__chrome_devtools_custom__click"));
        let read = descriptors
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "fs_read_text_file")
            .unwrap();
        assert_eq!(read["parameters"]["required"], json!(["path"]));
        assert_eq!(read["parameters"]["additionalProperties"], false);
    }

    #[test]
    fn bearwire_browser_prompt_includes_forwarded_mcp_tools() {
        let context = json!({
            "adapter": { "direct_tools": {} },
            "mcp": {
                "client_tools": [{
                    "name": "mcp__chrome_devtools_custom__click",
                    "description": "Click",
                    "parameters": { "type": "object", "properties": { "uid": { "type": "string" } }, "required": ["uid"] }
                }]
            }
        });
        let descriptors = client_tool_descriptors_from_context(Some(&context), Some("ask"));
        let names = descriptor_names(&descriptors);
        assert!(names.contains(&"mcp__chrome_devtools_custom__click"));
    }
}
