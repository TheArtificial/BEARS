use axum::http::HeaderMap;
use serde_json::{json, Value};

use bearwire_protocol::{methods::ConversationHistoryRequest, surface::SurfaceHistoryEvent};
use den_http::errors::CustomError;
use den_runtime::bearwire_events;
use den_service::{client_sessions, conversation::persistence, DenState};

use crate::auth::authenticated_bear;
use crate::methods::parse_params;

pub(crate) async fn conversation_history_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    conversation_history_like_result(state, headers, params, "conversation_history", "messages")
        .await
}

pub(crate) async fn conversation_surface_history_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    // ponytail: surface replay currently merges structured conversation rows with a small
    // allowlist of persisted BearWire events. The ceiling is pagination/order across independent
    // sequences; replace this with a dedicated persisted surface-event stream when Phase 3 grows.
    conversation_history_like_result(
        state,
        headers,
        params,
        "conversation_surface_history",
        "surface_events",
    )
    .await
}

async fn conversation_history_like_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
    response_kind: &str,
    records_key: &str,
) -> Result<Value, CustomError> {
    let (_user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: ConversationHistoryRequest = parse_params(params)?;
    let conversation_id = request.conversation_id;
    let before_sequence_no = request.before;
    let limit = request.limit.clamp(1, 100);

    let Some(conversation) =
        persistence::get_conversation_for_external_id(&state.sqlx_pool, bear.id, &conversation_id)
            .await?
    else {
        let mut response = json!({
            "kind": response_kind,
            "conversation_id": conversation_id,
            "has_more": false,
            "next_before": null,
            "missing": true,
        });
        response[records_key] = json!([]);
        return Ok(response);
    };

    let rows = persistence::list_messages_page(
        &state.sqlx_pool,
        conversation.id,
        before_sequence_no,
        limit,
    )
    .await?;
    let has_more = rows.len() >= limit as usize;
    let next_before = rows.last().map(|row| row.sequence_no.to_string());
    let mut messages = rows
        .iter()
        .rev()
        .filter_map(|row| {
            let message = row.to_user_history_record()?;
            let kind = message.kind.as_str();
            match kind {
                "tool_call" => Some(json!(SurfaceHistoryEvent::ToolCall {
                    id: message
                        .message_id
                        .or_else(|| Some(message.sequence_no.to_string())),
                    role: Some(message.role),
                    tool_call_id: message.tool_call_id?,
                    tool_name: message.tool_name?,
                    status: message.status?,
                    arguments: message.arguments,
                    created_at: Some(message.created_at.to_string()),
                })),
                "tool_result" => {
                    let text = (!message.content.trim().is_empty()).then(|| {
                        den_runtime::agent_assist::sanitize_visible_transcript_text(
                            &message.content,
                        )
                    });
                    Some(json!(SurfaceHistoryEvent::ToolResult {
                        id: message
                            .message_id
                            .or_else(|| Some(message.sequence_no.to_string())),
                        role: Some(message.role),
                        tool_call_id: message.tool_call_id?,
                        tool_name: message.tool_name?,
                        status: message.status?,
                        text,
                        raw_output: message.raw_output,
                        created_at: Some(message.created_at.to_string()),
                    }))
                }
                _ => {
                    let text = den_runtime::agent_assist::sanitize_visible_transcript_text(
                        &message.content,
                    );
                    if text.trim().is_empty() {
                        return None;
                    }
                    Some(json!(SurfaceHistoryEvent::Message {
                        id: message
                            .message_id
                            .or_else(|| Some(message.sequence_no.to_string())),
                        role: message.role,
                        text,
                        created_at: Some(message.created_at.to_string()),
                    }))
                }
            }
        })
        .collect::<Vec<_>>();

    if records_key == "surface_events" {
        if let Some(session) = client_sessions::find_latest_for_bear_conversation(
            &state.sqlx_pool,
            bear.id,
            &conversation_id,
        )
        .await?
        {
            messages.insert(
                0,
                json!(SurfaceHistoryEvent::SessionInfoUpdate {
                    id: Some(format!("session-info:{}", session.client_session_id)),
                    role: Some("system".to_string()),
                    session_id: Some(session.client_session_id.clone()),
                    title: session.conversation_title.clone(),
                    title_updated_at: session
                        .conversation_title_updated_at
                        .map(|value| value.to_string()),
                    current_mode: Some(session.current_mode.clone()),
                    created_at: Some(
                        session
                            .conversation_title_updated_at
                            .unwrap_or(session.updated_at)
                            .to_string(),
                    ),
                }),
            );

            let surface_event_rows = bearwire_events::list_bearwire_events_after(
                &state.sqlx_pool,
                &session.client_session_id,
                None,
                limit,
            )
            .await?;
            for row in surface_event_rows {
                if row.event_type == "session_info_update" {
                    let title = row
                        .event
                        .data
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|title| !title.is_empty());
                    let updated_at = row.event.data.get("updated_at").and_then(Value::as_str);
                    if title.is_none() && updated_at.is_none() {
                        continue;
                    }
                    let event_id = row
                        .event
                        .event_id
                        .unwrap_or_else(|| format!("bearwire:{}", row.id));
                    messages.push(json!(SurfaceHistoryEvent::SessionInfoUpdate {
                        id: Some(event_id),
                        role: Some("system".to_string()),
                        session_id: Some(session.client_session_id.clone()),
                        title: title.map(str::to_string),
                        title_updated_at: updated_at.map(str::to_string),
                        current_mode: None,
                        created_at: Some(row.created_at.to_string()),
                    }));
                    continue;
                }

                if row.event_type != "message.reasoning.delta" {
                    continue;
                }
                let delta = row
                    .event
                    .data
                    .get("delta")
                    .or_else(|| row.event.data.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if delta.is_empty() {
                    continue;
                }
                let source = row
                    .event
                    .data
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("provider_reasoning")
                    .to_string();
                let replay_policy = row
                    .event
                    .data
                    .get("replay_policy")
                    .and_then(Value::as_str)
                    .unwrap_or("none")
                    .to_string();
                if replay_policy != "thought" {
                    continue;
                }
                let event_id = row
                    .event
                    .event_id
                    .unwrap_or_else(|| format!("bearwire:{}", row.id));
                messages.push(json!(SurfaceHistoryEvent::ReasoningDelta {
                    id: Some(event_id),
                    role: Some("assistant".to_string()),
                    text: delta.to_string(),
                    source: Some(source),
                    replay_policy: Some(replay_policy),
                    created_at: Some(row.created_at.to_string()),
                }));
            }
        }
    }

    if records_key == "surface_events" {
        messages.sort_by(|left, right| {
            let left_created = left
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let right_created = right
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or_default();
            left_created.cmp(right_created)
        });
    }

    let mut response = json!({
        "kind": response_kind,
        "conversation_id": conversation_id,
        "has_more": has_more,
        "next_before": next_before,
    });
    response[records_key] = json!(messages);
    Ok(response)
}
