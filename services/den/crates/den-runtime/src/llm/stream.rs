use serde_json::Value;

use crate::runtime_contracts::{
    RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent,
};

fn delta_assistant_text(delta: &Value) -> Option<String> {
    for key in ["content", "text"] {
        if let Some(text) = delta.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            return Some(text.to_string());
        }
    }
    None
}

/// Accumulates OpenAI streaming tool-call argument fragments keyed by tool-call index.
#[derive(Debug, Default)]
pub struct OpenAiStreamAccumulator {
    tool_names: std::collections::HashMap<String, String>,
    tool_ids: std::collections::HashMap<String, String>,
    tool_args: std::collections::HashMap<String, String>,
    finish_reason: Option<String>,
    saw_tool_calls: bool,
}

#[derive(Debug, Default)]
pub struct OpenAiStreamParseResult {
    pub events: Vec<RuntimeStreamEvent>,
    pub finish_reason: Option<String>,
}

impl OpenAiStreamAccumulator {
    pub fn ingest_sse_data_line(&mut self, json: &Value) -> OpenAiStreamParseResult {
        let mut out = OpenAiStreamParseResult::default();
        if let Some(error) = json.get("error") {
            let message = error
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("LLM provider error")
                .to_string();
            out.events.push(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::Error {
                    message,
                    detail: Some(error.to_string()),
                    error_type: error
                        .get("type")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    request_id: None,
                    context: None,
                },
            ));
            return out;
        }

        let choice = json
            .pointer("/choices/0")
            .or_else(|| json.get("choice"));
        let Some(choice) = choice else {
            return out;
        };

        if let Some(delta) = choice.get("delta") {
            if let Some(content) = delta_assistant_text(delta) {
            out.events.push(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::AssistantTextDelta {
                    text: content,
                },
            ));
        }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                self.saw_tool_calls = true;
                for item in tool_calls {
                    let index_key = tool_call_index_key(item.get("index").unwrap_or(&Value::Null));
                    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                        self.tool_ids.insert(index_key.clone(), id.to_string());
                    }
                    if let Some(name) = item
                        .pointer("/function/name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        self.tool_names.insert(index_key.clone(), name.to_string());
                    }
                    if let Some(fragment) = item
                        .pointer("/function/arguments")
                        .and_then(|v| v.as_str())
                    {
                        self.tool_args
                            .entry(index_key)
                            .or_default()
                            .push_str(fragment);
                    }
                }
            }
        }

        if let Some(reason) = choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            self.finish_reason = Some(reason.to_string());
            out.finish_reason = Some(reason.to_string());
            out.events.extend(self.terminal_events_for_finish(reason));
        }

        out
    }

    fn terminal_events_for_finish(&self, finish_reason: &str) -> Vec<RuntimeStreamEvent> {
        match finish_reason {
            "tool_calls" => self
                .tool_ids
                .keys()
                .chain(self.tool_names.keys())
                .chain(self.tool_args.keys())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .map(|index_key| {
                    let tool_call_id = self
                        .tool_ids
                        .get(index_key)
                        .cloned()
                        .unwrap_or_else(|| format!("call-{index_key}"));
                    let tool_name = self
                        .tool_names
                        .get(index_key)
                        .cloned()
                        .unwrap_or_default();
                    let arguments = self
                        .tool_args
                        .get(index_key)
                        .map(|raw| parse_tool_arguments(raw))
                        .unwrap_or_else(|| Value::Object(Default::default()));
                    RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
                        tool_call_id,
                        tool_name,
                        title: None,
                        kind: Some("function".to_string()),
                        arguments,
                        approval_request_id: None,
                        approval_required: false,
                        approval_reason: None,
                        run_id: None,
                    })
                })
                .collect(),
            "stop" => vec![RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::TurnCompleted { turn: None },
            )],
            "length" => vec![RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::TurnFailed {
                    turn: None,
                    category: RuntimeErrorCategory::BackendProtocol,
                    message: "Model stopped due to length limit".to_string(),
                },
            )],
            other => vec![RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed {
                turn: None,
                category: RuntimeErrorCategory::BackendProtocol,
                message: format!("Model finished with unexpected reason: {other}"),
            })],
        }
    }

    /// True once the provider emitted a terminal `finish_reason` so the byte stream
    /// adapter can detach without waiting for upstream TCP close.
    pub fn should_detach_upstream(&self) -> bool {
        self.finish_reason.is_some()
    }

    pub fn flush_end_of_stream(&mut self) -> Vec<RuntimeStreamEvent> {
        if self.saw_tool_calls && self.finish_reason.is_none() {
            return self.terminal_events_for_finish("tool_calls");
        }
        if self.finish_reason.is_none() {
            return vec![RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::TurnCompleted { turn: None },
            )];
        }
        Vec::new()
    }
}

pub fn openai_sse_chunk_to_runtime_events(chunk_body: &[u8]) -> Result<Vec<RuntimeStreamEvent>, den_core::DenError> {
    let mut events = Vec::new();
    let text = std::str::from_utf8(chunk_body).map_err(|_| {
        den_core::DenError::System("invalid UTF-8 in LLM SSE chunk".to_string())
    })?;
    let mut accumulator = OpenAiStreamAccumulator::default();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !line.starts_with("data:") {
            continue;
        }
        let data = line.strip_prefix("data:").unwrap_or(line).trim();
        if data.is_empty() || data == "[DONE]" {
            if data == "[DONE]" {
                events.extend(accumulator.flush_end_of_stream());
            }
            continue;
        }
        let json = serde_json::from_str::<Value>(data).map_err(|e| {
            den_core::DenError::System(format!("invalid LLM SSE JSON: {e}"))
        })?;
        let parsed = accumulator.ingest_sse_data_line(&json);
        events.extend(parsed.events);
    }
    Ok(events)
}

/// Parse a single SSE event body (as used by the byte-stream adapter) into runtime events.
pub fn openai_sse_event_body_to_runtime_events(
    body: &[u8],
) -> Result<Vec<RuntimeStreamEvent>, den_core::DenError> {
    let mut accumulator = OpenAiStreamAccumulator::default();
    openai_sse_frame_to_runtime_events(&mut accumulator, body)
}

/// Parse one SSE frame into runtime events, preserving tool-call state across frames.
pub fn openai_sse_frame_to_runtime_events(
    accumulator: &mut OpenAiStreamAccumulator,
    body: &[u8],
) -> Result<Vec<RuntimeStreamEvent>, den_core::DenError> {
    let text = std::str::from_utf8(body).map_err(|_| {
        den_core::DenError::System("invalid UTF-8 in LLM SSE frame".to_string())
    })?;
    let mut events = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        let data = rest.strip_prefix(' ').unwrap_or(rest).trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            events.extend(accumulator.flush_end_of_stream());
            continue;
        }
        let json = serde_json::from_str::<Value>(data).map_err(|e| {
            den_core::DenError::System(format!("invalid LLM SSE JSON: {e}"))
        })?;
        let parsed = accumulator.ingest_sse_data_line(&json);
        events.extend(parsed.events);
    }
    Ok(events)
}

fn tool_call_index_key(value: &Value) -> String {
    value
        .as_u64()
        .map(|index| index.to_string())
        .or_else(|| value.as_i64().map(|index| index.to_string()))
        .or_else(|| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "0".to_string())
}

fn parse_tool_arguments(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| {
        if raw.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            Value::String(raw.to_string())
        }
    })
}
