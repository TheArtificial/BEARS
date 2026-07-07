use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{json, Value};

use den_http::errors::CustomError;
use den_service::{conversation::persistence, DenState};

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
    let (_user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: ConversationHistoryRequest = parse_params(params)?;
    let conversation_id = request.conversation_id;
    let before_sequence_no = request.before;
    let limit = request.limit.clamp(1, 100);

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

    Ok(json!({
        "kind": "conversation_history",
        "conversation_id": conversation_id,
        "messages": messages,
        "has_more": has_more,
        "next_before": next_before,
    }))
}
