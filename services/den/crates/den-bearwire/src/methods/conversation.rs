use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{json, Value};

use den_http::errors::CustomError;
use den_runtime::bearwire_events;
use den_service::{client_sessions, conversation::persistence, DenState};

use crate::auth::authenticated_bear;
use crate::methods::{
    deserialize_optional_i64_from_value, deserialize_required_string, parse_params,
};

#[derive(Debug, Deserialize)]
struct ConversationHistoryRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    conversation_id: String,
    #[serde(
        default,
        alias = "before_sequence_no",
        deserialize_with = "deserialize_optional_i64_from_value"
    )]
    before: Option<i64>,
    #[serde(default = "default_history_limit")]
    limit: i64,
}

fn default_history_limit() -> i64 {
    50
}

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
            let mut record = json!({
                "id": message.message_id.or_else(|| Some(message.sequence_no.to_string())),
                "kind": message.kind,
                "role": message.role,
                "created_at": message.created_at,
            });
            match record.get("kind").and_then(Value::as_str) {
                Some("tool_call") => {
                    record["tool_call_id"] = json!(message.tool_call_id);
                    record["tool_name"] = json!(message.tool_name);
                    record["status"] = json!(message.status);
                    record["arguments"] = message.arguments;
                    Some(record)
                }
                Some("tool_result") => {
                    record["tool_call_id"] = json!(message.tool_call_id);
                    record["tool_name"] = json!(message.tool_name);
                    record["status"] = json!(message.status);
                    record["raw_output"] = message.raw_output;
                    if !message.content.trim().is_empty() {
                        record["text"] =
                            json!(den_runtime::agent_assist::sanitize_visible_transcript_text(
                                &message.content
                            ));
                    }
                    Some(record)
                }
                _ => {
                    let text = den_runtime::agent_assist::sanitize_visible_transcript_text(
                        &message.content,
                    );
                    if text.trim().is_empty() {
                        return None;
                    }
                    record["text"] = json!(text);
                    Some(record)
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
            let mut session_event = json!({
                "id": format!("session-info:{}", session.client_session_id),
                "kind": "session_info_update",
                "role": "system",
                "session_id": session.client_session_id,
                "current_mode": session.current_mode,
                "created_at": session
                    .conversation_title_updated_at
                    .unwrap_or(session.updated_at),
            });
            if let Some(title) = session.conversation_title {
                session_event["title"] = json!(title);
            }
            if let Some(updated_at) = session.conversation_title_updated_at {
                session_event["title_updated_at"] = json!(updated_at);
            }
            messages.insert(0, session_event);

            let surface_event_rows = bearwire_events::list_bearwire_events_after(
                &state.sqlx_pool,
                &session.client_session_id,
                None,
                limit,
            )
            .await?;
            for row in surface_event_rows {
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
                if replay_policy == "none" {
                    continue;
                }
                let event_id = row
                    .event
                    .event_id
                    .unwrap_or_else(|| format!("bearwire:{}", row.id));
                messages.push(json!({
                    "id": event_id,
                    "kind": "reasoning_delta",
                    "role": "assistant",
                    "text": delta,
                    "source": source,
                    "replay_policy": replay_policy,
                    "created_at": row.created_at,
                }));
            }
        }
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
