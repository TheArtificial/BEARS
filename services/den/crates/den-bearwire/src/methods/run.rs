use std::{
    fmt,
    time::{Duration, Instant},
};

use axum::http::HeaderMap;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, types::time::OffsetDateTime};
use uuid::Uuid;

use bearwire_protocol::{
    methods::{RunCancelRequest, RunStartRequest},
    wire::{BearWireEvent, ResourceRef},
};
use den_http::errors::CustomError;
use den_protocol::RoleRuntimeBinding;
use den_runtime::{
    bearwire_events,
    native_runtime::start_native_profile_turn_event_stream,
    runtime::bearwire_projection::wire::runtime_stream_event_to_bearwire_events,
    runtime_error_ux::{log_sample, run_failure_projection, runtime_event_history_marker},
    surface_projection::bearwire_client_method_for_action,
    turn_obligations,
    turn_runner::TurnStartRequest,
    turn_runs, turn_steps,
};
use den_service::{
    DenState,
    bears::{BearProfile, db as bears_db},
    bifrost::BifrostCatalogEntry,
    client_sessions,
    conversation::events::{
        CanonicalConversationRecord, ConversationEventProvenance, canonical_persistence_context,
        persist_canonical_conversation_record,
    },
};

use crate::auth::authenticated_bear;
use crate::methods::{DEFAULT_CLIENT, parse_params};

const BEARWIRE_EAGER_PREFIX_DRIVE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Deserialize)]
struct RunStateRequest {
    bear_slug: String,
    run_id: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

fn den_owned_tool_call(tool_name: &str) -> bool {
    matches!(
        den_runtime::turn_waits::resolve_tool_execution_owner(tool_name),
        Ok(den_runtime::turn_waits::ToolExecutionOwner::Den)
    )
}

// Runtime status/error UX policy is surface-agnostic. Keep product copy,
// model-continuity summaries, and marker wording in den-runtime::runtime_error_ux.
// BearWire should only transport/persist those projections for this wire method.
async fn bear_display_name(pool: &sqlx::PgPool, bear_id: uuid::Uuid) -> String {
    bears_db::get_bear(pool, bear_id)
        .await
        .ok()
        .flatten()
        .map(|bear| bear.name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "The bear".to_string())
}

async fn persist_visible_runtime_marker(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    marker_kind: &str,
    marker_text: String,
    mut metadata: serde_json::Value,
) {
    let Ok(Some(session)) =
        client_sessions::find_for_user_bear_session_id(pool, user_id, bear_id, session_id).await
    else {
        return;
    };
    let conversation_id = session
        .resolved_conversation_id
        .clone()
        .unwrap_or_else(|| session.conversation_id.clone());
    let provenance = ConversationEventProvenance::client_session(session_id.to_string());
    metadata["source"] = json!("den.bearwire");
    metadata["event"] = json!("runtime_marker");
    metadata["scope_id"] = json!(run_id);
    metadata["request_id"] = json!(format!("{run_id}:{marker_kind}"));
    metadata["marker_kind"] = json!(marker_kind);
    metadata["run_id"] = json!(run_id);
    let _ = persist_canonical_conversation_record(
        &canonical_persistence_context(
            pool.clone(),
            bear_id,
            Some(user_id),
            conversation_id,
            Some(session_id.to_string()),
            Some(run_id.to_string()),
            provenance.scope_id,
            false,
        ),
        &CanonicalConversationRecord::visible_assistant_message(marker_text, metadata, None),
    )
    .await;
}

fn client_tool_descriptors_from_context_with_authority(
    client_context: Option<&Value>,
    authority: &den_core::client_tools::TurnAuthority,
) -> Value {
    let context = client_context.unwrap_or(&Value::Null);
    let mut descriptors = Vec::new();
    for tool in den_core::client_tools::ClientToolName::all() {
        if *tool == den_core::client_tools::ClientToolName::McpCallTool
            || !authority.allows_tool(*tool)
        {
            continue;
        }
        let descriptor = tool.descriptor();
        if !adapter_supports_tool(context, descriptor) {
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

fn workspace_roots_from_client_context(client_context: Option<&Value>) -> Vec<String> {
    client_context
        .and_then(|context| context.get("workspace_roots"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedRunModelSource {
    ConversationExplicit,
    ConversationAuto,
    ProfileDefault,
    BearDefault,
    SystemDefault,
}

impl ResolvedRunModelSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ConversationExplicit => "conversation_explicit",
            Self::ConversationAuto => "conversation_auto",
            Self::ProfileDefault => "profile_default",
            Self::BearDefault => "bear_default",
            Self::SystemDefault => "system_default",
        }
    }
}

impl fmt::Display for ResolvedRunModelSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

struct ResolvedRunModel {
    handle: String,
    provider_model_id: String,
    /// Set authoritatively by `preflight_pair_run_model` from the catalog
    /// snapshot; `resolve_pair_run_model` leaves it at a placeholder.
    api_style: den_llm::LlmApiStyle,
    source: ResolvedRunModelSource,
}

/// Placeholder used while resolving model identity; overwritten by preflight.
const RESOLVE_PLACEHOLDER_API_STYLE: den_llm::LlmApiStyle =
    den_llm::LlmApiStyle::ChatCompletionsStream;

fn provider_model_id_for_den_handle(handle: &str) -> String {
    den_llm::model_registry::provider_model_id_for_handle(handle)
        .unwrap_or_else(|| handle.trim())
        .to_string()
}

fn available_model_matches(
    model: &den_service::bifrost::BifrostModelMetadata,
    resolved: &ResolvedRunModel,
) -> bool {
    // Either of the catalog model's identifiers matching either resolved identifier is a hit.
    let model_ids = [model.handle.as_str(), model.model.as_str()];
    let resolved_ids = [
        resolved.handle.as_str(),
        resolved.provider_model_id.as_str(),
    ];
    model_ids
        .iter()
        .any(|model_id| resolved_ids.contains(model_id))
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

fn pair_api_style_for_catalog_support(
    supports_responses_api: Option<bool>,
) -> den_llm::LlmApiStyle {
    match supports_responses_api {
        Some(false) => den_llm::LlmApiStyle::ChatCompletionsStream,
        Some(true) | None => den_llm::LlmApiStyle::ResponsesStream,
    }
}

fn ensure_pair_model_capabilities(
    entry: &BifrostCatalogEntry,
    model_handle: &str,
) -> Result<(), CustomError> {
    if entry.supports_tools == Some(false) {
        return Err(CustomError::ValidationError(format!(
            "selected model {model_handle} does not support tool calling, which is required for Pair runs"
        )));
    }
    Ok(())
}

fn unknown_capability_metadata(entry: &BifrostCatalogEntry) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if entry.supports_tools.is_none() {
        missing.push("supports_tools");
    }
    if entry.supports_responses_api.is_none() {
        missing.push("supports_responses_api");
    }
    if entry.supports_vision.is_none() {
        missing.push("supports_vision");
    }
    missing
}

async fn resolve_pair_run_model(
    state: &DenState,
    bear: &den_service::bears::Bear,
    conversation_id: &str,
    stance: BearProfile,
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
                        source: ResolvedRunModelSource::ConversationExplicit,
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
                    source: ResolvedRunModelSource::ConversationAuto,
                });
            }
        }
    }

    if let Some(model) = bears_db::profile_model_setting(&state.sqlx_pool, bear.id, stance).await? {
        let handle = den_llm::normalize_llm_model_handle(&model);
        let provider_model_id = provider_model_id_for_den_handle(&handle);
        return Ok(ResolvedRunModel {
            api_style: RESOLVE_PLACEHOLDER_API_STYLE,
            provider_model_id,
            handle,
            source: ResolvedRunModelSource::ProfileDefault,
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
            source: ResolvedRunModelSource::BearDefault,
        });
    }

    let handle = den_llm::normalize_llm_model_handle(&state.config.default_llm_model);
    let provider_model_id = provider_model_id_for_den_handle(&handle);
    Ok(ResolvedRunModel {
        api_style: RESOLVE_PLACEHOLDER_API_STYLE,
        provider_model_id,
        handle,
        source: ResolvedRunModelSource::SystemDefault,
    })
}

async fn preflight_pair_run_model(
    state: &DenState,
    bear: &den_service::bears::Bear,
    session_id: &str,
    conversation_id: &str,
    stance: BearProfile,
) -> Result<ResolvedRunModel, CustomError> {
    let resolved = resolve_pair_run_model(state, bear, conversation_id, stance).await?;
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
            tracing::error!(
                error = %err,
                bear_id = %bear.id,
                "Bear-scoped Bifrost catalog refresh before Pair preflight failed"
            );
            return Err(CustomError::System(format!(
                "Bifrost model catalog validation failed before run start: {err}"
            )));
        }
    };
    let available = snapshot.models_vec();
    let catalog_entry = snapshot.resolve(&resolved.handle).ok_or_else(|| {
        CustomError::ValidationError(format!(
            "selected model {} is not present in the Bifrost catalog; available models: {}",
            resolved.handle,
            available_model_sample(&available)
        ))
    })?;
    ensure_pair_model_capabilities(catalog_entry, &resolved.handle)?;
    let unknown_capabilities = unknown_capability_metadata(catalog_entry);
    if !unknown_capabilities.is_empty() {
        tracing::warn!(
            session_id,
            bear_id = %bear.id,
            conversation_id,
            model_handle = %resolved.handle,
            unknown_capabilities = %unknown_capabilities.join(", "),
            catalog_source = %snapshot.source,
            "Bifrost omitted optional model capability metadata; using runtime fallbacks"
        );
    }
    let resolved = ResolvedRunModel {
        api_style: pair_api_style_for_catalog_support(catalog_entry.supports_responses_api),
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
            model_selection_source = %resolved.source,
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
        model_selection_source = %resolved.source,
        api_style = %resolved.api_style.as_str(),
        available_models = %available_model_sample(&available),
        catalog_stale = snapshot.stale,
        catalog_fetched_at = ?snapshot.fetched_at,
        "BearWire model preflight did not find selected model in catalog snapshot; proceeding and letting Bifrost execution validate"
    );
    Ok(resolved)
}

fn adapter_supports_tool(
    client_context: &Value,
    descriptor: &den_core::client_tools::ClientToolDescriptor,
) -> bool {
    std::iter::once(descriptor.provider_name)
        .chain(descriptor.provider_aliases.iter().copied())
        .any(|name| {
            client_context
                .pointer(&format!("/adapter/direct_tools/{name}/supported"))
                .and_then(Value::as_bool)
                .or_else(|| {
                    client_context
                        .pointer(&format!("/direct_tools/{name}"))
                        .and_then(Value::as_bool)
                })
                .unwrap_or(false)
        })
}

async fn run_is_terminal(pool: &sqlx::PgPool, run_id: &str) -> bool {
    turn_runs::get_run(pool, run_id)
        .await
        .ok()
        .flatten()
        .is_some_and(|run| matches!(run.state.as_str(), "completed" | "failed" | "cancelled"))
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
    if run_is_terminal(pool, run_id).await {
        tracing::debug!(
            session_id = %session_id,
            run_id = %run_id,
            kind,
            "skipping BearWire run.progress for terminal run"
        );
        return;
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeStreamBoundary {
    Continue,
    ClientWait,
    Terminal,
}

pub(crate) fn runtime_stream_boundary(
    event: &den_protocol::RuntimeStreamEvent,
) -> RuntimeStreamBoundary {
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
    match event {
        RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::TurnCompleted { .. }
            | RuntimeSemanticEvent::TurnFailed { .. }
            | RuntimeSemanticEvent::TurnCancelled { .. }
            | RuntimeSemanticEvent::Error { .. },
        ) => RuntimeStreamBoundary::Terminal,
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunPaused { .. }) => {
            RuntimeStreamBoundary::ClientWait
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_name,
            approval_required,
            approval_request_id,
            ..
        }) => {
            let approval_wait = *approval_required
                && approval_request_id
                    .as_deref()
                    .is_some_and(|id| !id.trim().is_empty());
            let den_owned = den_owned_tool_call(tool_name);
            if approval_wait || !den_owned {
                RuntimeStreamBoundary::ClientWait
            } else {
                RuntimeStreamBoundary::Continue
            }
        }
        _ => RuntimeStreamBoundary::Continue,
    }
}

fn runtime_event_is_terminal_or_wait(event: &den_protocol::RuntimeStreamEvent) -> bool {
    runtime_stream_boundary(event) != RuntimeStreamBoundary::Continue
}

pub(crate) fn runtime_event_kind(event: &den_protocol::RuntimeStreamEvent) -> &'static str {
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
    match event {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { .. }) => {
            "assistant_text_delta"
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ReasoningTextDelta { .. }) => {
            "reasoning_text_delta"
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
        RuntimeStreamEvent::ProviderActivity => "provider_activity",
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => "untranslated_provider_event",
    }
}

fn canonical_client_waiting_ids(event: &BearWireEvent) -> Option<(&str, &str)> {
    let permission_id = event
        .data
        .get("permission")
        .and_then(|permission| permission.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    let tool_call_id = event
        .data
        .get("tool_call")
        .and_then(|tool_call| tool_call.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    Some((permission_id, tool_call_id))
}

async fn append_answerable_client_waiting_event(
    pool: &sqlx::PgPool,
    session_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    run_id: &str,
    mut event: BearWireEvent,
    obligation: &turn_obligations::TurnObligationRow,
) -> Result<(), den_core::DenError> {
    let Some(expected_client_method) =
        bearwire_client_method_for_action(&obligation.expected_responder_action)
    else {
        tracing::warn!(
            session_id = %session_id,
            run_id = %run_id,
            obligation_id = %obligation.id,
            expected_responder_action = %obligation.expected_responder_action,
            "refusing to append client.waiting event for obligation without BearWire client method"
        );
        return Ok(());
    };
    if obligation.expected_responder_action != "permission_decision" {
        tracing::warn!(
            session_id = %session_id,
            run_id = %run_id,
            obligation_id = %obligation.id,
            expected_responder_action = %obligation.expected_responder_action,
            "refusing to append client.waiting event for non-permission obligation"
        );
        return Ok(());
    }
    let Some(permission_id) = obligation
        .permission_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        tracing::warn!(
            session_id = %session_id,
            run_id = %run_id,
            obligation_id = %obligation.id,
            "refusing to append client.waiting event without persisted permission id"
        );
        return Ok(());
    };
    let Some(tool_call_id) = obligation
        .tool_call_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        tracing::warn!(
            session_id = %session_id,
            run_id = %run_id,
            obligation_id = %obligation.id,
            "refusing to append client.waiting event without persisted tool call id"
        );
        return Ok(());
    };

    let Some((event_permission_id, event_tool_call_id)) = canonical_client_waiting_ids(&event)
    else {
        tracing::warn!(
            session_id = %session_id,
            run_id = %run_id,
            obligation_id = %obligation.id,
            "refusing to append client.waiting event without canonical permission.id/tool_call.id"
        );
        return Ok(());
    };
    if event_permission_id != permission_id {
        tracing::warn!(
            session_id = %session_id,
            run_id = %run_id,
            obligation_id = %obligation.id,
            expected_permission_id = %permission_id,
            event_permission_id = %event_permission_id,
            "refusing to append client.waiting event with malformed canonical permission id"
        );
        return Ok(());
    }
    if event_tool_call_id != tool_call_id {
        tracing::warn!(
            session_id = %session_id,
            run_id = %run_id,
            obligation_id = %obligation.id,
            expected_tool_call_id = %tool_call_id,
            event_tool_call_id = %event_tool_call_id,
            "refusing to append client.waiting event with malformed canonical tool_call id"
        );
        return Ok(());
    }

    event.data["obligation_id"] = json!(obligation.id.to_string());
    event.data["expected_responder_action"] = json!(obligation.expected_responder_action);
    event.data["expected_client_method"] = json!(expected_client_method);
    event.data["turn_step_id"] = json!(obligation.turn_step_id.map(|id| id.to_string()));
    event.resource_refs.push(ResourceRef::new(
        "client_obligation",
        obligation.id.to_string(),
    ));
    event.bear_id = Some(bear_id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.to_string());
    if event.run_id.is_none() {
        event.run_id = Some(run_id.to_string());
    }
    bearwire_events::append_bearwire_event(pool, session_id, Some(bear_id), Some(user_id), event)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn persist_tool_call_requested_transactionally(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    request_id: Uuid,
    started_at: Option<Instant>,
    tool_call_id: &str,
    tool_name: &str,
    title: &Option<String>,
    kind: &Option<String>,
    arguments: &Value,
    approval_request_id: &Option<String>,
    approval_required: bool,
    approval_reason: &Option<String>,
    event_run_id: &Option<String>,
) -> Result<(), den_core::DenError> {
    let persisted = den_runtime::turn_waits::persist_bearwire_tool_call_wait_transactionally(
        pool,
        den_runtime::turn_waits::PersistToolCallWaitInput {
            session_id,
            run_id,
            bear_id,
            user_id,
            request_id,
            tool_call_id,
            tool_name,
            title: title.as_deref(),
            kind: kind.as_deref(),
            arguments,
            approval_request_id: approval_request_id.as_deref(),
            approval_required,
            approval_reason: approval_reason.as_deref(),
            event_run_id: event_run_id.as_deref(),
        },
    )
    .await?;

    if let (Some(started_at), Some(obligation)) = (started_at, persisted.obligation.as_ref()) {
        let (kind, text) = if persisted.effective_approval_required {
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
                "approval_required": persisted.effective_approval_required,
                "approval_request_id": approval_request_id,
                "request_id": request_id,
                "turn_step_id": persisted.turn_step_id,
                "obligation_id": obligation.id,
                "event_sequence": persisted.event_sequence,
            }),
        )
        .await;
    }
    Ok(())
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
    if run_is_terminal(pool, run_id).await {
        tracing::debug!(
            session_id = %session_id,
            run_id = %run_id,
            event_kind = runtime_event_kind(&runtime_event),
            "skipping BearWire runtime event for terminal run"
        );
        return;
    }
    let bear_name = bear_display_name(pool, bear_id).await;
    let history_marker = runtime_event_history_marker(&bear_name, &runtime_event);
    if let den_protocol::RuntimeStreamEvent::Semantic(
        den_protocol::RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            title,
            kind,
            arguments,
            approval_request_id,
            approval_required,
            approval_reason,
            run_id: event_run_id,
        },
    ) = &runtime_event
    {
        match persist_tool_call_requested_transactionally(
            pool,
            session_id,
            run_id,
            bear_id,
            user_id,
            request_id,
            started_at,
            tool_call_id,
            tool_name,
            title,
            kind,
            arguments,
            approval_request_id,
            *approval_required,
            approval_reason,
            event_run_id,
        )
        .await
        {
            Ok(()) => return,
            Err(err) if den_runtime::turn_waits::descriptor_resolution_failed(&err) => {
                tracing::warn!(
                    error = %err,
                    session_id = %session_id,
                    run_id = %run_id,
                    tool_call_id = %tool_call_id,
                    tool_name = %tool_name,
                    "BearWire tool-call descriptor resolution failed; failing run instead of creating an ambiguous client wait"
                );
                persist_run_failed(
                    pool,
                    session_id,
                    run_id,
                    bear_id,
                    user_id,
                    "descriptor_resolution_failed",
                    err.to_string(),
                    Some(json!({
                        "tool_call_id": tool_call_id,
                        "tool_name": tool_name,
                        "request_id": request_id,
                    })),
                )
                .await;
                return;
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    session_id = %session_id,
                    run_id = %run_id,
                    tool_call_id = %tool_call_id,
                    "transactional BearWire tool-call persistence failed; falling back to legacy persistence path"
                );
            }
        }
    }

    let terminal_event = runtime_event_is_terminal(&runtime_event);
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
    if let Some(marker) = history_marker {
        persist_visible_runtime_marker(
            pool,
            session_id,
            run_id,
            bear_id,
            user_id,
            &marker.kind,
            marker.text,
            marker.metadata,
        )
        .await;
    }
    if terminal_event {
        return;
    }
    for mut event in runtime_stream_event_to_bearwire_events(runtime_event) {
        if event.event_type == "client.waiting" {
            if let Some(obligation) = active_obligation.as_ref() {
                if let Err(err) = append_answerable_client_waiting_event(
                    pool, session_id, bear_id, user_id, run_id, event, obligation,
                )
                .await
                {
                    tracing::warn!(
                        error = %err,
                        session_id = %session_id,
                        run_id = %run_id,
                        obligation_id = %obligation.id,
                        "failed to append answerable client.waiting BearWire event"
                    );
                }
            } else {
                tracing::warn!(
                    session_id = %session_id,
                    run_id = %run_id,
                    "runtime emitted client.waiting event without persisted BearWire obligation; dropping unanswerable event"
                );
            }
            continue;
        }
        if event.event_type == "permission.requested" && active_obligation.is_none() {
            tracing::warn!(
                session_id = %session_id,
                run_id = %run_id,
                "runtime emitted permission.requested event without persisted BearWire obligation; dropping unanswerable event"
            );
            continue;
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

pub(crate) async fn fail_run_lifecycle(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    reason: &str,
    message: String,
    context: Option<serde_json::Value>,
) {
    let bear_name = bear_display_name(pool, bear_id).await;
    let projection = run_failure_projection(reason, &message, run_id, &bear_name, context);
    let user_message = projection.user_message.as_deref();
    let context = projection.diagnostic_context.clone();
    tracing::warn!(
        session_id,
        run_id,
        bear_id = %bear_id,
        user_id,
        reason,
        user_message = user_message,
        error_message = %log_sample(&message),
        "BearWire run failed"
    );
    let mut event = BearWireEvent::ephemeral(
        "run.failed",
        json!({
            "run_id": run_id,
            "message": message,
            "user_message": user_message,
            "reason": reason,
            "context": context,
        }),
    );
    event.bear_id = Some(bear_id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.to_string());
    event.run_id = Some(run_id.to_string());
    let finished = turn_runs::finish_run_with_bearwire_event(
        pool,
        session_id,
        run_id,
        bear_id,
        user_id,
        turn_runs::TurnRunState::Failed,
        Some(reason),
        event,
    )
    .await
    .unwrap_or_else(|err| {
        tracing::error!(session_id, run_id, reason, error = %err, "failed to atomically persist BearWire run failure");
        None
    });
    if finished.is_none() {
        return;
    }
    record_work_run_outcome_if_bound(
        pool,
        session_id,
        run_id,
        "failed",
        Some(json!({ "category": reason, "message": message.clone() })),
    )
    .await;
    if let Ok(Some(session)) =
        client_sessions::find_for_user_bear_session_id(pool, user_id, bear_id, session_id).await
    {
        let conversation_id = session
            .resolved_conversation_id
            .clone()
            .unwrap_or_else(|| session.conversation_id.clone());
        let provenance = ConversationEventProvenance::client_session(session_id.to_string());
        let content_json = projection.content.clone();
        let marker_kind = content_json.get("kind").cloned().unwrap_or(Value::Null);
        let marker_retryable = content_json
            .get("retryable")
            .cloned()
            .unwrap_or(Value::Null);
        let _ = persist_canonical_conversation_record(
            &canonical_persistence_context(
                pool.clone(),
                bear_id,
                Some(user_id),
                conversation_id,
                Some(session_id.to_string()),
                Some(run_id.to_string()),
                provenance.scope_id,
                false,
            ),
            &CanonicalConversationRecord::model_visible_hidden_assistant_message(
                projection.model_summary.clone(),
                content_json,
                None,
            ),
        )
        .await;
        if let Some(marker) = projection.history_marker.clone() {
            persist_visible_runtime_marker(
                pool,
                session_id,
                run_id,
                bear_id,
                user_id,
                "operational_outcome",
                marker,
                json!({
                    "reason": reason,
                    "kind": marker_kind,
                    "retryable": marker_retryable,
                }),
            )
            .await;
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
    context: Option<serde_json::Value>,
) {
    fail_run_lifecycle(
        pool, session_id, run_id, bear_id, user_id, reason, message, context,
    )
    .await;
}

#[derive(Debug, Clone)]
struct SettledRunLifecycle {
    run: Option<turn_runs::TurnRunRow>,
    stream_run_ids: Vec<String>,
    cancelled_stream: bool,
    cancelled_tool_turn: bool,
    settled_obligations: u64,
    event_sequence: Option<i64>,
}

impl SettledRunLifecycle {
    fn settled(&self) -> bool {
        self.run.is_some() || self.cancelled_stream || self.cancelled_tool_turn
    }
}

async fn settle_active_run_for_session(
    state: &DenState,
    session_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    reason: &str,
    superseded_by_run_id: Option<&str>,
) -> Result<SettledRunLifecycle, CustomError> {
    let stream_cancel = state.turn_cancellations.cancel_session(session_id);
    let active_turn = state.tool_turns.cancel_active_turn(session_id);
    let active_run = turn_runs::active_run_for_session(&state.sqlx_pool, session_id).await?;
    let stream_run_ids = stream_cancel
        .as_ref()
        .map(|turn| turn.run_ids.clone())
        .unwrap_or_default();
    let cancelled_stream = stream_cancel.is_some();
    let cancelled_tool_turn = active_turn.is_some();
    let mut settled_obligations = 0;
    let mut event_sequence = None;
    if let Some(run) = &active_run {
        let mut event = BearWireEvent::ephemeral(
            "run.cancelled",
            json!({
                "session_id": session_id,
                "cancelled": true,
                "run_ids": stream_run_ids.clone(),
                "run_id": run.run_id,
                "reason": reason,
                "superseded_by_run_id": superseded_by_run_id,
                "cancelled_stream": cancelled_stream,
                "cancelled_tool_turn": cancelled_tool_turn,
            }),
        );
        event.bear_id = Some(bear_id.to_string());
        event.human_id = Some(user_id.to_string());
        event.session_id = Some(session_id.to_string());
        event.run_id = Some(run.run_id.clone());
        if let Some(finished) = turn_runs::finish_run_with_bearwire_event(
            &state.sqlx_pool,
            session_id,
            &run.run_id,
            bear_id,
            user_id,
            turn_runs::TurnRunState::Cancelled,
            Some(reason),
            event,
        )
        .await?
        {
            settled_obligations = finished.settled_obligations;
            event_sequence = Some(finished.event_sequence);
            record_work_run_outcome_if_bound(
                &state.sqlx_pool,
                session_id,
                &run.run_id,
                "cancelled",
                None,
            )
            .await;
        }
    } else if cancelled_stream || cancelled_tool_turn {
        let mut event = BearWireEvent::ephemeral(
            "run.cancelled",
            json!({
                "session_id": session_id,
                "cancelled": true,
                "run_ids": stream_run_ids.clone(),
                "reason": reason,
                "superseded_by_run_id": superseded_by_run_id,
                "cancelled_stream": cancelled_stream,
                "cancelled_tool_turn": cancelled_tool_turn,
            }),
        );
        event.bear_id = Some(bear_id.to_string());
        event.human_id = Some(user_id.to_string());
        event.session_id = Some(session_id.to_string());
        event_sequence = Some(
            bearwire_events::append_bearwire_event(
                &state.sqlx_pool,
                session_id,
                Some(bear_id),
                Some(user_id),
                event,
            )
            .await?
            .sequence_no,
        );
    }
    Ok(SettledRunLifecycle {
        run: active_run,
        stream_run_ids,
        cancelled_stream,
        cancelled_tool_turn,
        settled_obligations,
        event_sequence,
    })
}

/// Work-run hook: when this session was bound by `work.checkout`, record the
/// terminal turn outcome and move the work run to `reporting` so the dispatch
/// worker harvests it. A no-op (one indexed lookup) for ordinary Pair sessions.
async fn record_work_run_outcome_if_bound(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    kind: &str,
    detail: Option<Value>,
) {
    let outcome = json!({
        "kind": kind,
        "run_id": run_id,
        "detail": detail,
    });
    match den_docket::work_runs::record_work_run_turn_outcome(pool, session_id, &outcome).await {
        Ok(Some(work_run)) => {
            tracing::info!(
                session_id,
                run_id,
                work_run_id = %work_run.id,
                outcome_kind = kind,
                "work run moved to reporting after terminal turn event"
            );
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(
                session_id,
                run_id,
                error = %err,
                "failed to record work run turn outcome"
            );
        }
    }
}

fn runtime_event_is_terminal(event: &den_protocol::RuntimeStreamEvent) -> bool {
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
    matches!(
        event,
        RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::TurnCompleted { .. }
                | RuntimeSemanticEvent::TurnFailed { .. }
                | RuntimeSemanticEvent::TurnCancelled { .. }
                | RuntimeSemanticEvent::Error { .. }
        )
    )
}

async fn finish_runtime_terminal_event(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    event: &den_protocol::RuntimeStreamEvent,
) {
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
    let (state, reason, outcome, detail) = match event {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. }) => (
            turn_runs::TurnRunState::Completed,
            "completed".to_string(),
            "completed",
            None,
        ),
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed {
            category,
            message,
            ..
        }) => {
            let category = category.as_str();
            (
                turn_runs::TurnRunState::Failed,
                category.to_string(),
                "failed",
                Some(json!({ "category": category, "message": message })),
            )
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCancelled { .. }) => (
            turn_runs::TurnRunState::Cancelled,
            "cancelled".to_string(),
            "cancelled",
            None,
        ),
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error {
            message,
            error_type,
            ..
        }) => (
            turn_runs::TurnRunState::Failed,
            error_type.clone().unwrap_or_else(|| "error".to_string()),
            "failed",
            Some(json!({ "category": error_type, "message": message })),
        ),
        _ => return,
    };
    let Some(mut terminal_event) = runtime_stream_event_to_bearwire_events(event.clone())
        .into_iter()
        .find(|event| {
            matches!(
                event.event_type.as_str(),
                "run.completed" | "run.failed" | "run.cancelled"
            )
        })
    else {
        tracing::error!(
            session_id,
            run_id,
            "terminal runtime event had no BearWire terminal projection"
        );
        return;
    };
    terminal_event.bear_id = Some(bear_id.to_string());
    terminal_event.human_id = Some(user_id.to_string());
    terminal_event.session_id = Some(session_id.to_string());
    terminal_event.run_id = Some(run_id.to_string());
    match turn_runs::finish_run_with_bearwire_event(
        pool,
        session_id,
        run_id,
        bear_id,
        user_id,
        state,
        Some(&reason),
        terminal_event,
    )
    .await
    {
        Ok(Some(_)) => {
            record_work_run_outcome_if_bound(pool, session_id, run_id, outcome, detail).await;
        }
        Ok(None) => {}
        Err(err) => {
            tracing::error!(session_id, run_id, error = %err, "failed to atomically persist terminal runtime event");
        }
    }
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
) -> Option<turn_obligations::TurnObligationRow> {
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
    if runtime_event_is_terminal(event) {
        finish_runtime_terminal_event(pool, session_id, run_id, bear_id, user_id, event).await;
        return None;
    }
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
            if den_owned_tool_call(tool_name) {
                return None;
            }
            let _ = turn_runs::transition_run(
                pool,
                run_id,
                turn_runs::TurnRunState::WaitingForClient,
                None,
            )
            .await;
            let turn_step_id = match turn_steps::ensure_active_step(pool, run_id).await {
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
                    turn_obligations::upsert_permission_decision_obligation_for_step(
                        pool,
                        run_id,
                        session_id,
                        turn_step_id,
                        permission_id,
                        Some(tool_call_id),
                        request_payload,
                    )
                    .await
                } else {
                    turn_obligations::upsert_tool_result_obligation_for_step(
                        pool,
                        run_id,
                        session_id,
                        turn_step_id,
                        tool_call_id,
                        None,
                        request_payload,
                    )
                    .await
                }
            } else {
                turn_obligations::upsert_tool_result_obligation_for_step(
                    pool,
                    run_id,
                    session_id,
                    turn_step_id,
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

        _ => None,
    }
}

pub(crate) async fn run_start_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: RunStartRequest = parse_params(params)?;
    let session_id = request.session_id;
    let prompt = request.prompt;
    let prompt_context = request.prompt_context;
    let client = request.client.unwrap_or_else(|| DEFAULT_CLIENT.to_string());
    let existing = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?;
    let conversation_id = request
        .conversation_id
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
    let cwd = request.cwd;
    let client_context = request.client_context;
    let workspace_roots = workspace_roots_from_client_context(client_context.as_ref());
    let requested_mode = request.requested_mode;
    // `TurnAuthority` is the single local authority seam for BearWire's tool
    // surface and prompt permission envelope. The concrete stance is resolved
    // below for runtime binding; BearWire sessions use Pair-equivalent tool
    // authority unless a later typed work-run authority input says otherwise.
    let turn_authority = den_core::client_tools::TurnAuthority::for_session_mode(
        den_core::BearStance::Pair,
        requested_mode.as_deref().unwrap_or("ask"),
        None,
    );
    let client_tools = client_tool_descriptors_from_context_with_authority(
        client_context.as_ref(),
        &turn_authority,
    );
    let read_only_runtime_context = turn_authority.read_only_runtime_context();
    // Stance signal: a session bound to a live work run via `work.checkout`
    // runs in the Work stance; every other BearWire session stays Pair.
    let stance =
        if den_docket::work_runs::get_live_work_run_by_session(&state.sqlx_pool, &session_id)
            .await?
            .is_some()
        {
            BearProfile::Work
        } else {
            BearProfile::Pair
        };
    let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, stance)
        .await?
        .ok_or_else(|| {
            CustomError::System(format!(
                "Bear {} profile binding not configured",
                stance.as_str()
            ))
        })?;
    let binding = RoleRuntimeBinding {
        binding_id,
        compatibility_backend: Some("native".to_string()),
    };
    let resolved_model =
        preflight_pair_run_model(state, &bear, &session_id, &upstream_target, stance).await?;
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
    if let Some(client_context) = client_context.as_ref() {
        client_sessions::update_adapter_environment(
            &state.sqlx_pool,
            user_id,
            bear.id,
            &session_id,
            client_context,
        )
        .await?;
    }

    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let superseded = settle_active_run_for_session(
        state,
        &session_id,
        bear.id,
        user_id,
        "superseded_by_new_run",
        Some(&run_id),
    )
    .await?;
    if superseded.settled() {
        tracing::info!(
            session_id = %session_id,
            new_run_id = %run_id,
            old_run_id = superseded.run.as_ref().map(|run| run.run_id.as_str()),
            cancelled_stream = superseded.cancelled_stream,
            cancelled_tool_turn = superseded.cancelled_tool_turn,
            settled_obligations = superseded.settled_obligations,
            event_sequence = superseded.event_sequence,
            "BearWire superseded active run before starting new run"
        );
    }

    let run =
        turn_runs::create_run(&state.sqlx_pool, &run_id, &session_id, bear.id, user_id).await?;
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

    let request_id = Uuid::new_v4();
    let (cancel_handle, mut cancel_rx) = state.turn_cancellations.register(
        session_id.clone(),
        request_id,
        Some(upstream_target.clone()),
    );
    let _ = cancel_handle.record_run_id(&run_id);

    let pool = state.sqlx_pool.clone();
    let config = state.config.clone();
    let memory_stores = state.memory_stores.clone();
    let bear_slug = bear.slug.clone();
    let bear_id = bear.id;
    let session_for_task = session_id.clone();
    let conversation_for_task = conversation_id.clone();
    let upstream_target_for_task = upstream_target.clone();
    let prompt_for_task = prompt.clone();
    let read_only_runtime_context_for_task = read_only_runtime_context.clone();
    let run_id_for_task = run_id.clone();
    let client_tools_for_task = client_tools.clone();
    let api_style_for_task = resolved_model.api_style;
    let (eager_prefix_tx, eager_prefix_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _cancel_handle = cancel_handle;
        let mut eager_prefix_tx = Some(eager_prefix_tx);
        let run_started_at = Instant::now();
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
        let _ = turn_runs::transition_run(
            &pool,
            &run_id_for_task,
            turn_runs::TurnRunState::Running,
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
        let stream_result = start_native_profile_turn_event_stream(
            TurnStartRequest {
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
                workspace_roots: Some(&workspace_roots),
                binding: &binding,
                conversation_selection: &conversation_for_task,
                upstream_target: &upstream_target_for_task,
                prompt: &prompt_for_task,
                prompt_context: prompt_context.clone(),
                client_tools: Some(client_tools_for_task),
                runtime_context: read_only_runtime_context_for_task.as_deref(),
                runtime_context_len: read_only_runtime_context_for_task
                    .as_deref()
                    .map(str::len)
                    .unwrap_or(0),
                stream_tokens: true,
                api_style: Some(api_style_for_task),
            },
            stance,
        )
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
                let mut provider_activity_seen = false;
                let mut terminal_or_wait_seen = false;
                let mut cancellation_seen = false;
                let idle_watchdog_timeout = crate::methods::client::continuation_watchdog_timeout();
                let handshake_timeout = den_runtime::agent_loop::native_llm_handshake_timeout();
                let first_event_watchdog_timeout =
                    crate::methods::client::continuation_first_event_watchdog_timeout(
                        handshake_timeout,
                        idle_watchdog_timeout,
                    );
                loop {
                    let watchdog_timeout = if first_event_seen || provider_activity_seen {
                        idle_watchdog_timeout
                    } else {
                        first_event_watchdog_timeout
                    };
                    tokio::select! {
                        changed = cancel_rx.changed() => {
                            if changed.is_ok() && *cancel_rx.borrow() {
                                cancellation_seen = true;
                                if let Some(tx) = eager_prefix_tx.take() {
                                    let _ = tx.send(());
                                }
                                tracing::info!(
                                    session_id = %session_for_task,
                                    run_id = %run_id_for_task,
                                    request_id = %request_id,
                                    "BearWire background run stream observed cancellation"
                                );
                                break;
                            }
                        }
                        timed = tokio::time::timeout(watchdog_timeout, stream.next()) => {
                            let item = match timed {
                                Ok(item) => item,
                                Err(_) => {
                                    if let Some(tx) = eager_prefix_tx.take() {
                                        let _ = tx.send(());
                                    }
                                    persist_run_failed(
                                        &pool,
                                        &session_for_task,
                                        &run_id_for_task,
                                        bear_id,
                                        user_id,
                                        "initial_stream_watchdog_timeout",
                                        if provider_activity_seen {
                                            format!("Provider activity stopped for {}ms before the run reached a terminal or client-wait event.", watchdog_timeout.as_millis())
                                        } else {
                                            format!("The provider did not begin streaming within the {}ms initial handshake window.", watchdog_timeout.as_millis())
                                        },
                                        Some(json!({
                                            "request_id": request_id,
                                            "provider_activity_seen": provider_activity_seen,
                                            "first_event_seen": first_event_seen,
                                            "watchdog_timeout_ms": watchdog_timeout.as_millis(),
                                        })),
                                    )
                                    .await;
                                    break;
                                }
                            };
                            let Some(item) = item else {
                                break;
                            };
                            match item {
                                Ok(den_protocol::RuntimeStreamEvent::ProviderActivity) => {
                                    provider_activity_seen = true;
                                    continue;
                                }
                                Ok(runtime_event) => {
                                    if runtime_event_is_terminal_or_wait(&runtime_event) {
                                        terminal_or_wait_seen = true;
                                    }
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
                                        None,
                                    )
                                    .await;
                                    break;
                                }
                            }
                        }
                    }
                }
                if !terminal_or_wait_seen && !cancellation_seen {
                    if let Some(tx) = eager_prefix_tx.take() {
                        let _ = tx.send(());
                    }
                    persist_run_failed(
                        &pool,
                        &session_for_task,
                        &run_id_for_task,
                        bear_id,
                        user_id,
                        "stream_ended_without_runtime_terminal",
                        if first_event_seen {
                            "The model stream ended after non-terminal runtime events but did not emit a tool request, completion, cancellation, or error.".to_string()
                        } else {
                            "The model stream ended without emitting any runtime event for BearWire to deliver.".to_string()
                        },
                        Some(json!({
                            "request_id": request_id,
                            "first_event_seen": first_event_seen,
                        })),
                    )
                    .await;
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
                    None,
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

async fn run_obligations_payload(
    pool: &sqlx::PgPool,
    run_id: &str,
) -> Result<Vec<Value>, CustomError> {
    let rows = sqlx::query(
        r"
        SELECT id, run_id, session_id, kind, expected_responder_action,
               tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
               created_at, updated_at, completed_at
        FROM turn_obligations
        WHERE run_id = $1
        ORDER BY created_at ASC, id ASC
        ",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let created_at: OffsetDateTime = row.get("created_at");
            let updated_at: OffsetDateTime = row.get("updated_at");
            let completed_at: Option<OffsetDateTime> = row.get("completed_at");
            json!({
                "id": row.get::<uuid::Uuid, _>("id"),
                "run_id": row.get::<String, _>("run_id"),
                "session_id": row.get::<String, _>("session_id"),
                "kind": row.get::<String, _>("kind"),
                "expected_responder_action": row.get::<String, _>("expected_responder_action"),
                "tool_call_id": row.get::<Option<String>, _>("tool_call_id"),
                "permission_id": row.get::<Option<String>, _>("permission_id"),
                "state": row.get::<String, _>("state"),
                "turn_step_id": row.get::<Option<uuid::Uuid>, _>("turn_step_id"),
                "request_payload": row.get::<Value, _>("request_payload"),
                "result_payload": row.get::<Option<Value>, _>("result_payload"),
                "created_at": created_at,
                "updated_at": updated_at,
                "completed_at": completed_at,
            })
        })
        .collect())
}

async fn run_results_payload(pool: &sqlx::PgPool, run_id: &str) -> Result<Vec<Value>, CustomError> {
    let rows = sqlx::query(
        r"
        SELECT id, run_id, obligation_kind, obligation_id, result_hash, payload_json, turn_step_id, created_at
        FROM turn_obligation_results
        WHERE run_id = $1
        ORDER BY created_at ASC, id ASC
        ",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let created_at: OffsetDateTime = row.get("created_at");
            json!({
                "id": row.get::<uuid::Uuid, _>("id"),
                "run_id": row.get::<String, _>("run_id"),
                "obligation_kind": row.get::<String, _>("obligation_kind"),
                "obligation_id": row.get::<String, _>("obligation_id"),
                "result_hash": row.get::<String, _>("result_hash"),
                "payload_json": row.get::<Value, _>("payload_json"),
                "turn_step_id": row.get::<Option<uuid::Uuid>, _>("turn_step_id"),
                "created_at": created_at,
            })
        })
        .collect())
}

async fn run_recent_events_payload(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    limit: i64,
) -> Result<Vec<Value>, CustomError> {
    let rows = sqlx::query(
        r"
        SELECT id, sequence_no, event_type, event_json, created_at
        FROM bearwire_events
        WHERE session_id = $1
          AND event_json->>'run_id' = $2
        ORDER BY sequence_no DESC
        LIMIT $3
        ",
    )
    .bind(session_id)
    .bind(run_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?;
    let mut events = rows
        .into_iter()
        .map(|row| {
            let created_at: OffsetDateTime = row.get("created_at");
            json!({
                "id": row.get::<uuid::Uuid, _>("id"),
                "sequence_no": row.get::<i64, _>("sequence_no"),
                "event_type": row.get::<String, _>("event_type"),
                "event": row.get::<Value, _>("event_json"),
                "created_at": created_at,
            })
        })
        .collect::<Vec<_>>();
    events.reverse();
    Ok(events)
}

pub(crate) async fn run_state_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: RunStateRequest = parse_params(params)?;
    if request.bear_slug != bear.slug {
        return Err(CustomError::Authorization(
            "bear_slug does not match authenticated Bear".to_string(),
        ));
    }
    let run = turn_runs::get_run(&state.sqlx_pool, &request.run_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("BearWire run not found".to_string()))?;
    if run.bear_id != bear.id || run.user_id != user_id {
        return Err(CustomError::Authorization(
            "run does not belong to authenticated Bear".to_string(),
        ));
    }
    if let Some(session_id) = request.session_id.as_deref() {
        if run.session_id != session_id {
            return Err(CustomError::Authorization(
                "run does not belong to requested session".to_string(),
            ));
        }
    }
    let obligations = run_obligations_payload(&state.sqlx_pool, &run.run_id).await?;
    let open_obligations = obligations
        .iter()
        .filter(|obligation| {
            obligation
                .get("state")
                .and_then(Value::as_str)
                .is_some_and(|state| matches!(state, "requested" | "waiting_for_client"))
        })
        .cloned()
        .collect::<Vec<_>>();
    let blocking_reason = turn_obligations::BlockingReason::from_expected_actions(
        open_obligations.iter().filter_map(|obligation| {
            obligation
                .get("expected_responder_action")
                .and_then(Value::as_str)
                .and_then(|action| {
                    turn_obligations::ExpectedResponderAction::try_from_storage(action).ok()
                })
        }),
    )
    .map(turn_obligations::BlockingReason::as_str);
    let results = run_results_payload(&state.sqlx_pool, &run.run_id).await?;
    let events = run_recent_events_payload(
        &state.sqlx_pool,
        &run.session_id,
        &run.run_id,
        request.limit.unwrap_or(50),
    )
    .await?;
    Ok(json!({
        "kind": "run_state",
        "run": run,
        "blocking_reason": blocking_reason,
        "open_obligations": open_obligations,
        "obligations": obligations,
        "results": results,
        "recent_events": events,
    }))
}

pub(crate) async fn run_cancel_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: RunCancelRequest = parse_params(params)?;
    let session_id = request.session_id;
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

    let settled = settle_active_run_for_session(
        state,
        &session.client_session_id,
        bear.id,
        user_id,
        "client_requested",
        None,
    )
    .await?;
    let cancelled = settled.settled();

    Ok(json!({
        "ok": true,
        "cancelled": cancelled,
        "session_id": session_id,
        "run_ids": settled.stream_run_ids,
        "run_id": settled.run.as_ref().map(|run| run.run_id.clone()),
        "cancelled_stream": settled.cancelled_stream,
        "cancelled_tool_turn": settled.cancelled_tool_turn,
        "settled_obligations": settled.settled_obligations,
        "event_sequence": settled.event_sequence,
        "reason": if cancelled { "client_requested" } else { "no_active_run" },
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
            source: ResolvedRunModelSource::ConversationExplicit,
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
    fn pair_api_style_prefers_responses_unless_catalog_explicitly_denies_support() {
        assert_eq!(
            pair_api_style_for_catalog_support(None),
            den_llm::LlmApiStyle::ResponsesStream
        );
        assert_eq!(
            pair_api_style_for_catalog_support(Some(true)),
            den_llm::LlmApiStyle::ResponsesStream
        );
        assert_eq!(
            pair_api_style_for_catalog_support(Some(false)),
            den_llm::LlmApiStyle::ChatCompletionsStream
        );
    }

    #[test]
    fn pair_model_capabilities_allow_unknown_tool_support() {
        let entry = BifrostCatalogEntry {
            available: true,
            provider: "openai".to_string(),
            provider_model_id: "gpt-5.6-terra".to_string(),
            gateway_handle: "openai/gpt-5.6-terra".to_string(),
            display_name: None,
            context_window: 128_000,
            max_output_tokens: Some(4096),
            supports_tools: None,
            supports_responses_api: None,
            supports_vision: None,
        };

        ensure_pair_model_capabilities(&entry, &entry.gateway_handle)
            .expect("unknown optional metadata must not block Pair");
    }

    #[test]
    fn pair_model_capabilities_reject_explicit_tool_denial() {
        let mut entry = BifrostCatalogEntry {
            available: true,
            provider: "openai".to_string(),
            provider_model_id: "gpt-no-tools".to_string(),
            gateway_handle: "openai/gpt-no-tools".to_string(),
            display_name: None,
            context_window: 128_000,
            max_output_tokens: Some(4096),
            supports_tools: Some(false),
            supports_responses_api: None,
            supports_vision: None,
        };

        let err = ensure_pair_model_capabilities(&entry, &entry.gateway_handle)
            .expect_err("explicit tool denial must block Pair");
        assert!(err.to_string().contains("does not support tool calling"));
        entry.supports_tools = Some(true);
        ensure_pair_model_capabilities(&entry, &entry.gateway_handle)
            .expect("explicit tool support must allow Pair");
    }

    #[test]
    fn unknown_capability_metadata_lists_unknown_fields() {
        let entry = BifrostCatalogEntry {
            available: true,
            provider: "openai".to_string(),
            provider_model_id: "gpt-5.6-terra".to_string(),
            gateway_handle: "openai/gpt-5.6-terra".to_string(),
            display_name: None,
            context_window: 128_000,
            max_output_tokens: Some(4096),
            supports_tools: Some(true),
            supports_responses_api: None,
            supports_vision: None,
        };

        assert_eq!(
            unknown_capability_metadata(&entry),
            vec!["supports_responses_api", "supports_vision"]
        );
    }

    #[test]
    fn canonical_client_waiting_ids_require_nested_ids() {
        let event = BearWireEvent::ephemeral(
            "client.waiting",
            json!({
                "tool_call": { "id": "call-1", "name": "web_fetch" },
                "permission": { "id": "perm-1" }
            }),
        );

        assert_eq!(
            canonical_client_waiting_ids(&event),
            Some(("perm-1", "call-1"))
        );
    }

    #[test]
    fn canonical_client_waiting_ids_reject_legacy_only_ids() {
        let event = BearWireEvent::ephemeral(
            "client.waiting",
            json!({
                "tool_call_id": "call-1",
                "permission_id": "perm-1"
            }),
        );

        assert_eq!(canonical_client_waiting_ids(&event), None);
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
        let authority = den_core::client_tools::TurnAuthority::for_session_mode(
            den_core::BearStance::Pair,
            "write",
            None,
        );
        let descriptors =
            client_tool_descriptors_from_context_with_authority(Some(&context), &authority);
        let names = descriptor_names(&descriptors);
        assert!(names.contains(&"fs_read_text_file"));
        assert!(names.contains(&"fs_find_paths"));
        assert!(names.contains(&"fs_edit_file"));
        assert!(names.contains(&"run_command"));
        assert!(!names.contains(&"terminal_run_command"));
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
        let authority = den_core::client_tools::TurnAuthority::for_session_mode(
            den_core::BearStance::Pair,
            "ask",
            None,
        );
        let descriptors =
            client_tool_descriptors_from_context_with_authority(Some(&context), &authority);
        let names = descriptor_names(&descriptors);
        assert!(names.contains(&"mcp__chrome_devtools_custom__click"));
    }
}
