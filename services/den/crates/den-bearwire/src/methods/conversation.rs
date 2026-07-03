use axum::http::HeaderMap;
use serde_json::{json, Value};

use den_http::errors::CustomError;
use den_service::{conversation::persistence, DenState};

use crate::auth::authenticated_bear;
use crate::methods::required_param_string;

pub(crate) async fn conversation_history_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (_user_id, bear) = authenticated_bear(state, headers, params).await?;
    let conversation_id = required_param_string(params, "conversation_id")?;
    let before_sequence_no = params
        .get("before")
        .or_else(|| params.get("before_sequence_no"))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
        });
    let limit = params
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(50)
        .clamp(1, 100);

    let Some(conversation) =
        persistence::get_conversation_for_external_id(&state.sqlx_pool, bear.id, &conversation_id)
            .await?
    else {
        return Ok(json!({
            "kind": "conversation_history",
            "conversation_id": conversation_id,
            "messages": [],
            "has_more": false,
            "next_before": null,
            "missing": true,
        }));
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
    let messages = rows
        .iter()
        .rev()
        .filter_map(|row| {
            let message = row.to_user_history_record()?;
            let text =
                den_runtime::agent_assist::sanitize_visible_transcript_text(&message.content);
            if text.trim().is_empty() {
                return None;
            }
            Some(json!({
                "id": message.message_id.or_else(|| Some(message.sequence_no.to_string())),
                "role": message.role,
                "text": text,
                "created_at": message.created_at,
            }))
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "kind": "conversation_history",
        "conversation_id": conversation_id,
        "messages": messages,
        "has_more": has_more,
        "next_before": next_before,
    }))
}
