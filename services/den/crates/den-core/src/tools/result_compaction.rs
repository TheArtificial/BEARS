use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MODEL_TOOL_RESULT_MAX_CHARS: usize = 24 * 1024;
pub const TOOL_RESULT_FIELD_MAX_CHARS: usize = 12 * 1024;

#[derive(Debug, Clone)]
pub struct CompactToolResult {
    pub payload: Value,
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Ok,
    Error,
    Timeout,
    Incomplete,
    Cancelled,
}

impl ToolResultStatus {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "ok" => Some(Self::Ok),
            "error" => Some(Self::Error),
            "timeout" | "timed_out" => Some(Self::Timeout),
            "incomplete" => Some(Self::Incomplete),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Incomplete => "incomplete",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClientToolResultInput {
    pub tool_call_id: String,
    pub tool_name: Option<String>,
    pub status: ToolResultStatus,
    pub content: Option<String>,
    pub structured_content: Value,
    pub error: Value,
}

impl ClientToolResultInput {
    pub fn new(
        tool_call_id: impl Into<String>,
        tool_name: Option<String>,
        status: ToolResultStatus,
        content: Option<String>,
        structured_content: Value,
        error: Value,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            status,
            content: content
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            structured_content,
            error,
        }
    }

    pub fn from_params(tool_call_id: &str, status: ToolResultStatus, params: &Value) -> Self {
        Self::new(
            tool_call_id.to_string(),
            params
                .get("tool_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            status,
            params
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string),
            params
                .get("structured_content")
                .cloned()
                .unwrap_or(Value::Null),
            params.get("error").cloned().unwrap_or(Value::Null),
        )
    }
}

fn preview_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        Value::Object(map) => map
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_string)
            .filter(|text| !text.is_empty())
            .or_else(|| {
                let rendered = value.to_string();
                Some(rendered).filter(|text| text != "{}" && text != "null" && text != "[]")
            }),
        Value::Null => None,
        other => {
            let rendered = other.to_string();
            Some(rendered).filter(|text| text != "null" && text != "[]")
        }
    }
}

fn bounded_summary(tool_name: Option<&str>, status: &str, preview: Option<&str>) -> String {
    let subject = tool_name.filter(|name| !name.is_empty()).unwrap_or("tool");
    match preview {
        Some(preview) if !preview.is_empty() => format!("Used {subject} ({status}): {preview}"),
        _ => format!("Used {subject} ({status})"),
    }
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

fn compaction_metadata(omitted_chars: usize, artifact_ref: Option<&str>) -> Value {
    let mut metadata = json!({
        "truncated": true,
        "omitted_chars": omitted_chars,
        "model_max_chars": MODEL_TOOL_RESULT_MAX_CHARS,
        "field_max_chars": TOOL_RESULT_FIELD_MAX_CHARS,
    });
    if let Some(artifact_ref) = artifact_ref {
        metadata["artifact_ref"] = json!(artifact_ref);
        metadata["read_tool"] = json!("tool_output_read");
    }
    metadata
}

pub fn compact_json_tool_result(value: Value) -> CompactToolResult {
    compact_json_tool_result_with_artifact(value, None)
}

pub fn compact_json_tool_result_with_artifact(
    value: Value,
    artifact_ref: Option<&str>,
) -> CompactToolResult {
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
                    compaction_metadata(omitted_chars, artifact_ref),
                );
            }
            other => {
                payload = json!({
                    "value": other.clone(),
                    "result_compaction": compaction_metadata(omitted_chars, artifact_ref)
                });
            }
        }
    }

    let mut content =
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    if content.chars().count() > MODEL_TOOL_RESULT_MAX_CHARS {
        let (text, _, _) = truncate_str(&content, MODEL_TOOL_RESULT_MAX_CHARS);
        content = text;
    }
    if truncated && !content.contains("tool result truncated") {
        content.push_str(&format!(
            "\n\n[tool result truncated for model context; omitted_chars={omitted_chars}]"
        ));
    }

    CompactToolResult {
        payload,
        content,
        truncated,
    }
}

pub fn compact_client_tool_result(input: &ClientToolResultInput) -> CompactToolResult {
    compact_client_tool_result_with_artifact(input, None)
}

pub fn compact_client_tool_result_with_artifact(
    input: &ClientToolResultInput,
    artifact_ref: Option<&str>,
) -> CompactToolResult {
    let mut truncated = false;
    let mut omitted_chars = 0usize;
    let tool_call_id = input.tool_call_id.as_str();
    let status = input.status.as_str();
    let (content, content_truncated, content_omitted) = compact_value_for_model(
        input
            .content
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        TOOL_RESULT_FIELD_MAX_CHARS,
    );
    truncated |= content_truncated;
    omitted_chars += content_omitted;
    let (structured_content, structured_truncated, structured_omitted) = compact_value_for_model(
        input.structured_content.clone(),
        TOOL_RESULT_FIELD_MAX_CHARS,
    );
    truncated |= structured_truncated;
    omitted_chars += structured_omitted;
    let (error, error_truncated, error_omitted) =
        compact_value_for_model(input.error.clone(), TOOL_RESULT_FIELD_MAX_CHARS);
    truncated |= error_truncated;
    omitted_chars += error_omitted;

    let mut payload = json!({
        "tool_call_id": tool_call_id,
        "status": status,
        "content": content,
        "structured_content": structured_content,
        "error": error,
    });
    if let Some(tool_name) = input.tool_name.as_deref() {
        payload["tool_name"] = json!(tool_name);
    }
    let tool_name = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let preview = payload
        .get("content")
        .and_then(preview_text)
        .or_else(|| payload.get("structured_content").and_then(preview_text))
        .or_else(|| payload.get("error").and_then(preview_text));
    if let Some(preview) = preview.as_ref() {
        let (preview, _, _) = truncate_str(preview, 512);
        payload["output_preview"] = json!(preview);
    }
    payload["output_summary"] = json!(bounded_summary(
        tool_name.as_deref(),
        status,
        preview.as_deref()
    ));
    let serialized_len = payload.to_string().chars().count();
    if serialized_len > MODEL_TOOL_RESULT_MAX_CHARS {
        truncated = true;
        omitted_chars += serialized_len.saturating_sub(MODEL_TOOL_RESULT_MAX_CHARS);
    }
    if truncated {
        payload["result_compaction"] = compaction_metadata(omitted_chars, artifact_ref);
    }

    let mut content_text = payload
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .or_else(|| payload.get("structured_content").and_then(preview_text))
        .or_else(|| payload.get("error").and_then(preview_text))
        .unwrap_or_default();
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
        truncated,
    }
}

#[cfg(test)]
mod tests;
