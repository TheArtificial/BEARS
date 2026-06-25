use serde_json::{json, Value};

pub const MODEL_TOOL_RESULT_MAX_CHARS: usize = 24 * 1024;
pub const TOOL_RESULT_FIELD_MAX_CHARS: usize = 12 * 1024;

#[derive(Debug, Clone)]
pub struct CompactToolResult {
    pub payload: Value,
    pub content: String,
}

pub fn truncate_str(value: &str, max_chars: usize) -> (String, bool, usize) {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return (value.to_string(), false, 0);
    }
    let omitted = char_count.saturating_sub(max_chars);
    (
        format!(
            "{}\n... truncated, omitted {omitted} characters",
            value.chars().take(max_chars).collect::<String>()
        ),
        true,
        omitted,
    )
}

pub fn compact_value_for_model(value: Value, max_chars: usize) -> (Value, bool, usize) {
    match value {
        Value::String(text) => {
            let (text, truncated, omitted_chars) = truncate_str(&text, max_chars);
            (Value::String(text), truncated, omitted_chars)
        }
        Value::Array(items) => {
            let mut remaining = max_chars;
            let mut truncated = false;
            let mut omitted_chars = 0usize;
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                if remaining == 0 {
                    truncated = true;
                    omitted_chars += item.to_string().chars().count();
                    continue;
                }
                let (item, item_truncated, item_omitted) = compact_value_for_model(item, remaining);
                let used = item.to_string().chars().count().min(remaining);
                remaining = remaining.saturating_sub(used);
                truncated |= item_truncated;
                omitted_chars += item_omitted;
                out.push(item);
            }
            (Value::Array(out), truncated, omitted_chars)
        }
        Value::Object(map) => {
            let mut remaining = max_chars;
            let mut truncated = false;
            let mut omitted_chars = 0usize;
            let mut out = serde_json::Map::new();
            for (key, item) in map {
                if remaining == 0 {
                    truncated = true;
                    omitted_chars += item.to_string().chars().count();
                    continue;
                }
                let (item, item_truncated, item_omitted) = compact_value_for_model(item, remaining);
                let used = item.to_string().chars().count().min(remaining);
                remaining = remaining.saturating_sub(used);
                truncated |= item_truncated;
                omitted_chars += item_omitted;
                out.insert(key, item);
            }
            (Value::Object(out), truncated, omitted_chars)
        }
        other => (other, false, 0),
    }
}

pub fn compact_json_tool_result(value: Value) -> CompactToolResult {
    let (mut payload, truncated, omitted_chars) =
        compact_value_for_model(value, MODEL_TOOL_RESULT_MAX_CHARS);
    let serialized_len = payload.to_string().chars().count();
    let mut truncated = truncated || serialized_len > MODEL_TOOL_RESULT_MAX_CHARS;
    let mut omitted_chars = omitted_chars
        + if serialized_len > MODEL_TOOL_RESULT_MAX_CHARS {
            serialized_len.saturating_sub(MODEL_TOOL_RESULT_MAX_CHARS)
        } else {
            0
        };
    if serialized_len > MODEL_TOOL_RESULT_MAX_CHARS {
        let (text, _, omitted) = truncate_str(&payload.to_string(), MODEL_TOOL_RESULT_MAX_CHARS);
        payload = json!({ "preview": text });
        truncated = true;
        omitted_chars += omitted;
    }
    if truncated {
        match &mut payload {
            Value::Object(map) => {
                map.insert(
                    "result_compaction".to_string(),
                    json!({
                        "truncated": true,
                        "omitted_chars": omitted_chars,
                        "model_max_chars": MODEL_TOOL_RESULT_MAX_CHARS,
                        "field_max_chars": TOOL_RESULT_FIELD_MAX_CHARS,
                    }),
                );
            }
            other => {
                payload = json!({
                    "value": other.clone(),
                    "result_compaction": {
                        "truncated": true,
                        "omitted_chars": omitted_chars,
                        "model_max_chars": MODEL_TOOL_RESULT_MAX_CHARS,
                        "field_max_chars": TOOL_RESULT_FIELD_MAX_CHARS,
                    }
                });
            }
        }
    }

    let mut content = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    if content.chars().count() > MODEL_TOOL_RESULT_MAX_CHARS {
        let (text, _, _) = truncate_str(&content, MODEL_TOOL_RESULT_MAX_CHARS);
        content = text;
    }
    if truncated && !content.contains("tool result truncated") {
        content.push_str(&format!(
            "\n\n[tool result truncated for model context; omitted_chars={omitted_chars}]"
        ));
    }

    CompactToolResult { payload, content }
}

pub fn compact_client_tool_result_params(
    tool_call_id: &str,
    status: &str,
    params: &Value,
) -> CompactToolResult {
    let mut truncated = false;
    let mut omitted_chars = 0usize;
    let (content, content_truncated, content_omitted) = compact_value_for_model(
        params.get("content").cloned().unwrap_or(Value::Null),
        TOOL_RESULT_FIELD_MAX_CHARS,
    );
    truncated |= content_truncated;
    omitted_chars += content_omitted;
    let (structured_content, structured_truncated, structured_omitted) = compact_value_for_model(
        params
            .get("structured_content")
            .cloned()
            .unwrap_or(Value::Null),
        TOOL_RESULT_FIELD_MAX_CHARS,
    );
    truncated |= structured_truncated;
    omitted_chars += structured_omitted;
    let (error, error_truncated, error_omitted) = compact_value_for_model(
        params.get("error").cloned().unwrap_or(Value::Null),
        TOOL_RESULT_FIELD_MAX_CHARS,
    );
    truncated |= error_truncated;
    omitted_chars += error_omitted;

    let mut payload = json!({
        "tool_call_id": tool_call_id,
        "status": status,
        "content": content,
        "structured_content": structured_content,
        "error": error,
    });
    let serialized_len = payload.to_string().chars().count();
    if serialized_len > MODEL_TOOL_RESULT_MAX_CHARS {
        truncated = true;
        omitted_chars += serialized_len.saturating_sub(MODEL_TOOL_RESULT_MAX_CHARS);
    }
    if truncated {
        payload["result_compaction"] = json!({
            "truncated": true,
            "omitted_chars": omitted_chars,
            "model_max_chars": MODEL_TOOL_RESULT_MAX_CHARS,
            "field_max_chars": TOOL_RESULT_FIELD_MAX_CHARS,
        });
    }

    let mut content_text = payload
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            payload
                .get("structured_content")
                .or_else(|| payload.get("error"))
                .map(|v| v.to_string())
                .unwrap_or_default()
        });
    if content_text.chars().count() > MODEL_TOOL_RESULT_MAX_CHARS {
        let (text, _, _) = truncate_str(&content_text, MODEL_TOOL_RESULT_MAX_CHARS);
        content_text = text;
    }
    if truncated && !content_text.contains("tool result truncated") {
        content_text.push_str(&format!(
            "\n\n[tool result truncated for model context; omitted_chars={omitted_chars}]"
        ));
    }

    CompactToolResult {
        payload,
        content: content_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_client_tool_result_truncates_large_content_for_model() {
        let long = "x".repeat(40 * 1024);
        let compacted = compact_client_tool_result_params(
            "call_large",
            "ok",
            &json!({
                "content": long,
                "structured_content": { "nested": "y".repeat(40 * 1024) },
            }),
        );

        assert_eq!(compacted.payload["tool_call_id"], "call_large");
        assert_eq!(compacted.payload["result_compaction"]["truncated"], true);
        assert!(compacted.content.contains("tool result truncated for model context"));
        assert!(compacted.content.chars().count() < 25 * 1024);
    }

    #[test]
    fn compact_json_tool_result_truncates_large_den_hosted_result() {
        let compacted = compact_json_tool_result(json!({
            "results": [{ "body": "x".repeat(40 * 1024) }]
        }));

        assert_eq!(compacted.payload["result_compaction"]["truncated"], true);
        assert!(compacted.content.contains("tool result truncated for model context"));
        assert!(compacted.content.chars().count() < 25 * 1024);
    }
}
