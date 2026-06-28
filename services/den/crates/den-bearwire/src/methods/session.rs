use axum::http::HeaderMap;
use serde_json::{json, Value};

use den_http::errors::CustomError;
use den_service::{client_sessions, bears::BearProfile, DenState};
use den_runtime::{
    bearwire_events,
    runtime::bearwire_projection::wire::BearWireEvent,
};

use crate::auth::{authenticate_for_bear_slug, authenticated_bear};
use crate::methods::{param_string, required_param_string};

pub(crate) async fn session_open_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let existing = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?;
    let client = param_string(params, "client").unwrap_or_else(|| "bearwire".to_string());
    let conversation_id = param_string(params, "conversation_id")
        .or_else(|| {
            existing
                .as_ref()
                .map(|session| session.conversation_id.clone())
        })
        .unwrap_or_else(|| format!("new-acp-{client}-{}", uuid::Uuid::new_v4().simple()));
    let resolved_conversation_id = existing
        .as_ref()
        .and_then(|session| session.resolved_conversation_id.clone());
    let runtime_session_id = param_string(params, "runtime_session_id")
        .or_else(|| {
            existing
                .as_ref()
                .map(|session| session.runtime_session_id.clone())
        })
        .unwrap_or_else(|| format!("bearwire:{}:{}", bear.id, session_id));
    let cwd = param_string(params, "cwd");
    let current_mode = param_string(params, "mode");
    client_sessions::upsert_session(
        &state.sqlx_pool,
        client_sessions::UpsertClientSession {
            user_id,
            bear_id: bear.id,
            bear_slug: bear.slug.clone(),
            client_session_id: session_id.clone(),
            runtime_session_id,
            conversation_id,
            resolved_conversation_id,
            client,
            cwd,
            current_mode,
        },
    )
    .await?;
    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?;
    let mut event = BearWireEvent::ephemeral(
        "session.opened",
        json!({
            "session_id": session_id,
            "bear_slug": bear.slug,
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
        "session": session,
        "event_sequence": persisted.sequence_no,
    }))
}

pub(crate) async fn session_close_result(
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
        return Ok(json!({ "ok": true, "closed": false, "session_id": session_id }));
    };
    client_sessions::mark_closed(&state.sqlx_pool, session.id).await?;
    let mut event = BearWireEvent::ephemeral(
        "session.closed",
        json!({
            "session_id": session_id,
            "bear_slug": bear.slug,
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
        "closed": true,
        "session_id": session_id,
        "event_sequence": persisted.sequence_no,
    }))
}

pub(crate) async fn session_state_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let Some(bear_slug) = params
        .get("bear_slug")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(json!({
            "status": "available",
            "note": "Provide bear_slug and optional session_id for authenticated BearWire session state.",
            "params": params,
        }));
    };
    let user_id = authenticate_for_bear_slug(state, headers, bear_slug).await?;
    if let Some(session_id) = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let session = client_sessions::find_for_user_bear_session(
            &state.sqlx_pool,
            user_id,
            bear_slug,
            session_id,
        )
        .await?;
        return Ok(json!({
            "kind": "single",
            "bear_slug": bear_slug,
            "session": session,
        }));
    }

    let include_closed = params
        .get("include_closed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let limit = params
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(50)
        .clamp(1, 100);
    let sessions = client_sessions::list_for_user_bear(
        &state.sqlx_pool,
        client_sessions::SessionListParams {
            user_id,
            bear_slug,
            include_closed,
            cwd_filter: None,
            limit,
            cursor_updated_at: None,
            cursor_id: None,
        },
    )
    .await?;
    Ok(json!({
        "kind": "list",
        "bear_slug": bear_slug,
        "sessions": sessions,
    }))
}

async fn session_model_payload(
    state: &DenState,
    user_id: i32,
    bear: &den_service::bears::Bear,
    session_id: &str,
) -> Result<Value, CustomError> {
    let session =
        client_sessions::find_for_user_bear_session(&state.sqlx_pool, user_id, &bear.slug, session_id)
            .await?
            .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let conversation_id = session
        .resolved_conversation_id
        .as_deref()
        .unwrap_or(&session.conversation_id);
    let conversation = den_service::conversation::persistence::ensure_conversation_for_external_id(
        &state.sqlx_pool,
        bear.id,
        Some(user_id),
        conversation_id,
        Some(&session.client_session_id),
        None,
    )
    .await?;
    let base_model = den_service::bears::db::resolve_model_for_profile(
        &state.sqlx_pool,
        bear,
        BearProfile::Pair,
        state.config.default_llm_model.as_str(),
    )
    .await?;
    let model_state = den_service::conversation::persistence::get_conversation_model_state(
        &state.sqlx_pool,
        conversation.id,
    )
    .await?;
    let effective_model =
        den_service::conversation::persistence::resolve_conversation_selected_model(
            &state.sqlx_pool,
            conversation.id,
        )
        .await?
        .unwrap_or(base_model);
    let model_options =
        den_service::model_selection::list_selectable_model_options(&state.sqlx_pool).await?;
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "conversation_id": conversation_id,
        "selection_mode": model_state.as_ref().map(|s| s.selection_mode.as_str()).unwrap_or("auto"),
        "requested_model": model_state.as_ref().and_then(|s| s.requested_model.clone()),
        "selected_model": model_state.as_ref().and_then(|s| s.selected_model.clone()),
        "effective_model": effective_model,
        "model_options": model_options,
    }))
}

pub(crate) async fn session_model_get_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    session_model_payload(state, user_id, &bear, &session_id).await
}

pub(crate) async fn session_model_set_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let mode = param_string(params, "selection_mode").unwrap_or_else(|| "auto".to_string());
    let requested_model = param_string(params, "model");
    let payload = session_model_payload(state, user_id, &bear, &session_id).await?;
    let options = payload
        .get("model_options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let selected = if mode.trim() == "explicit" {
        let raw = requested_model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CustomError::ValidationError("model is required for explicit selection".to_string())
            })?;
        if den_llm::model_registry::is_routing_wildcard_model_handle(raw) {
            return Err(CustomError::ValidationError(
                "routing wildcards are not selectable models".to_string(),
            ));
        }
        let resolved = den_llm::model_registry::resolve_model_handle(raw);
        let available = options.iter().any(|option| {
            let handle = option.get("handle").and_then(Value::as_str).unwrap_or("");
            handle == raw
                || resolved == Some(handle)
                || den_llm::model_registry::resolve_model_handle(handle) == resolved
        });
        if !available {
            return Err(CustomError::ValidationError(
                "model must be configured as a selectable Den model".to_string(),
            ));
        }
        Some(resolved.unwrap_or(raw).to_string())
    } else {
        None
    };
    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let conversation_id = session
        .resolved_conversation_id
        .as_deref()
        .unwrap_or(&session.conversation_id);
    let conversation = den_service::conversation::persistence::ensure_conversation_for_external_id(
        &state.sqlx_pool,
        bear.id,
        Some(user_id),
        conversation_id,
        Some(&session.client_session_id),
        None,
    )
    .await?;
    den_service::conversation::persistence::set_conversation_model_state(
        &state.sqlx_pool,
        conversation.id,
        if selected.is_some() {
            "explicit"
        } else {
            "auto"
        },
        selected.as_deref(),
        selected.as_deref(),
        Some(if selected.is_some() {
            "acp_selected"
        } else {
            "inherit_stance_or_bear_default"
        }),
    )
    .await?;

    let mut event = BearWireEvent::ephemeral(
        "model.selection.changed",
        json!({
            "session_id": session_id,
            "conversation_id": conversation_id,
            "selection_mode": if selected.is_some() { "explicit" } else { "auto" },
            "selected_model": selected,
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    let _ = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;

    session_model_payload(state, user_id, &bear, &session_id).await
}
