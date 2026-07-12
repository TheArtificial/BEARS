use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use den_llm::LlmRequestTelemetry;
use serde_json::Value;

use den_protocol::{RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent};

fn delta_assistant_text(delta: &Value) -> Option<String> {
    for key in ["content", "text"] {
        if let Some(text) = delta
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            return Some(text.to_string());
        }
    }
    None
}

fn delta_reasoning_text(delta: &Value) -> Option<String> {
    for key in ["reasoning", "reasoning_content", "thinking", "thought"] {
        if let Some(text) = delta
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
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

#[derive(Debug, Clone)]
pub struct OpenAiStreamDiagnostics {
    telemetry: Option<LlmRequestTelemetry>,
    started_at: Instant,
    sse_frame_count: u64,
    json_frame_count: u64,
    tool_delta_count: u64,
    emitted_runtime_event_count: u64,
    tool_arg_lengths: HashMap<String, usize>,
}

impl OpenAiStreamDiagnostics {
    pub fn new(telemetry: Option<LlmRequestTelemetry>) -> Self {
        Self {
            telemetry,
            started_at: Instant::now(),
            sse_frame_count: 0,
            json_frame_count: 0,
            tool_delta_count: 0,
            emitted_runtime_event_count: 0,
            tool_arg_lengths: HashMap::new(),
        }
    }

    fn telemetry(&self) -> Option<&LlmRequestTelemetry> {
        self.telemetry.as_ref()
    }

    pub fn observe_sse_frame(&mut self, byte_len: usize) {
        self.sse_frame_count = self.sse_frame_count.saturating_add(1);
        let telemetry = self.telemetry();
        tracing::trace!(
            target: "den.llm.stream",
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            sse_frame_count = self.sse_frame_count,
            frame_bytes = byte_len,
            elapsed_ms = self.started_at.elapsed().as_millis(),
            "LLM stream SSE frame parsed"
        );
    }

    pub fn observe_json_frame(&mut self, json: &Value) {
        self.json_frame_count = self.json_frame_count.saturating_add(1);
        let finish_reason = json
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str);
        let has_tool_delta = json
            .pointer("/choices/0/delta/tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty());
        let has_assistant_delta = json
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty());
        let telemetry = self.telemetry();
        tracing::trace!(
            target: "den.llm.stream",
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            json_frame_count = self.json_frame_count,
            has_tool_delta,
            has_assistant_delta,
            finish_reason,
            elapsed_ms = self.started_at.elapsed().as_millis(),
            "LLM stream JSON frame parsed"
        );
    }

    fn observe_tool_delta(
        &mut self,
        index_key: &str,
        id: Option<&str>,
        name: Option<&str>,
        fragment_len: usize,
        total_args_len: usize,
    ) {
        self.tool_delta_count = self.tool_delta_count.saturating_add(1);
        self.tool_arg_lengths
            .insert(index_key.to_string(), total_args_len);
        let telemetry = self.telemetry();
        tracing::trace!(
            target: "den.llm.stream",
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            tool_delta_count = self.tool_delta_count,
            tool_index = index_key,
            tool_call_id = id,
            tool_name = name,
            argument_fragment_bytes = fragment_len,
            total_argument_bytes = total_args_len,
            elapsed_ms = self.started_at.elapsed().as_millis(),
            "LLM stream tool-call delta observed"
        );
    }

    fn observe_finish_reason(&mut self, finish_reason: &str, emitted_events: usize) {
        let telemetry = self.telemetry();
        tracing::debug!(
            target: "den.llm.stream",
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            finish_reason,
            emitted_events,
            tool_delta_count = self.tool_delta_count,
            tool_arg_lengths = ?self.tool_arg_lengths,
            elapsed_ms = self.started_at.elapsed().as_millis(),
            "LLM stream finish reason observed"
        );
    }

    pub fn observe_emitted_events(&mut self, events: &[RuntimeStreamEvent]) {
        if events.is_empty() {
            return;
        }
        self.emitted_runtime_event_count = self
            .emitted_runtime_event_count
            .saturating_add(events.len() as u64);
        let telemetry = self.telemetry();
        if tracing::enabled!(target: "den.llm.stream", tracing::Level::TRACE) {
            let event_kinds = events.iter().map(runtime_event_kind).collect::<Vec<_>>();
            tracing::trace!(
                target: "den.llm.stream",
                request_id = telemetry.and_then(|t| t.request_id.as_deref()),
                run_id = telemetry.and_then(|t| t.run_id.as_deref()),
                session_id = telemetry.and_then(|t| t.session_id.as_deref()),
                conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
                emitted_runtime_event_count = self.emitted_runtime_event_count,
                emitted_this_frame = events.len(),
                event_kinds = ?event_kinds,
                elapsed_ms = self.started_at.elapsed().as_millis(),
                "LLM stream runtime events emitted"
            );
        } else {
            tracing::debug!(
                target: "den.llm.stream",
                request_id = telemetry.and_then(|t| t.request_id.as_deref()),
                run_id = telemetry.and_then(|t| t.run_id.as_deref()),
                session_id = telemetry.and_then(|t| t.session_id.as_deref()),
                conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
                emitted_runtime_event_count = self.emitted_runtime_event_count,
                emitted_this_frame = events.len(),
                elapsed_ms = self.started_at.elapsed().as_millis(),
                "LLM stream runtime events emitted"
            );
        }
    }
}

fn runtime_event_kind(event: &RuntimeStreamEvent) -> &'static str {
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
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error { .. }) => "error",
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
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => "untranslated_provider_event",
    }
}

impl OpenAiStreamAccumulator {
    pub fn ingest_sse_data_line(&mut self, json: &Value) -> OpenAiStreamParseResult {
        self.ingest_sse_data_line_with_diagnostics(json, None)
    }

    pub fn ingest_sse_data_line_with_diagnostics(
        &mut self,
        json: &Value,
        mut diagnostics: Option<&mut OpenAiStreamDiagnostics>,
    ) -> OpenAiStreamParseResult {
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.observe_json_frame(json);
        }
        let mut out = OpenAiStreamParseResult::default();
        if let Some(error) = json.get("error") {
            let message = error
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("LLM provider error")
                .to_string();
            out.events
                .push(RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error {
                    message,
                    detail: Some(error.to_string()),
                    error_type: error
                        .get("type")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    request_id: None,
                    context: None,
                }));
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.observe_emitted_events(&out.events);
            }
            return out;
        }

        let choice = json.pointer("/choices/0").or_else(|| json.get("choice"));
        let Some(choice) = choice else {
            return out;
        };

        if let Some(delta) = choice.get("delta") {
            if let Some(content) = delta_assistant_text(delta) {
                out.events.push(RuntimeStreamEvent::Semantic(
                    RuntimeSemanticEvent::AssistantTextDelta { text: content },
                ));
            }
            if let Some(reasoning) = delta_reasoning_text(delta) {
                out.events.push(RuntimeStreamEvent::Semantic(
                    RuntimeSemanticEvent::ReasoningTextDelta { text: reasoning },
                ));
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                self.saw_tool_calls = true;
                for item in tool_calls {
                    let index_key = tool_call_index_key(item.get("index").unwrap_or(&Value::Null));
                    let id = item.get("id").and_then(|v| v.as_str());
                    if let Some(id) = id {
                        self.tool_ids.insert(index_key.clone(), id.to_string());
                    }
                    let name = item
                        .pointer("/function/name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty());
                    if let Some(name) = name {
                        self.tool_names.insert(index_key.clone(), name.to_string());
                    }
                    let fragment = item.pointer("/function/arguments").and_then(|v| v.as_str());
                    if let Some(fragment) = fragment {
                        self.tool_args
                            .entry(index_key.clone())
                            .or_default()
                            .push_str(fragment);
                    }
                    if let Some(diagnostics) = diagnostics.as_deref_mut() {
                        let total_args_len =
                            self.tool_args.get(&index_key).map(String::len).unwrap_or(0);
                        diagnostics.observe_tool_delta(
                            &index_key,
                            id,
                            name,
                            fragment.map(str::len).unwrap_or(0),
                            total_args_len,
                        );
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
            let terminal_events = self.terminal_events_for_finish(reason);
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.observe_finish_reason(reason, terminal_events.len());
            }
            out.events.extend(terminal_events);
        }

        if let Some(diagnostics) = diagnostics {
            diagnostics.observe_emitted_events(&out.events);
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
                    let tool_name = self.tool_names.get(index_key).cloned().unwrap_or_default();
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
            other => vec![RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::TurnFailed {
                    turn: None,
                    category: RuntimeErrorCategory::BackendProtocol,
                    message: format!("Model finished with unexpected reason: {other}"),
                },
            )],
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

#[derive(Debug, Default)]
pub struct ResponsesStreamAccumulator {
    tool_names: HashMap<String, String>,
    tool_call_ids: HashMap<String, String>,
    tool_args: HashMap<String, String>,
    emitted_tool_call_keys: HashSet<String>,
    completed: bool,
    saw_tool_call: bool,
}

#[derive(Debug, Clone)]
pub struct ResponsesStreamDiagnostics {
    telemetry: Option<LlmRequestTelemetry>,
    started_at: Instant,
    sse_frame_count: u64,
    json_frame_count: u64,
    zero_runtime_event_frame_count: u64,
    consecutive_zero_runtime_event_frame_count: u64,
    emitted_runtime_event_count: u64,
    event_type_counts: HashMap<String, u64>,
    last_event_type: Option<String>,
    tool_arg_lengths: HashMap<String, usize>,
    tool_names: HashMap<String, String>,
    tool_call_ids: HashMap<String, String>,
}

impl ResponsesStreamDiagnostics {
    pub fn new(telemetry: Option<LlmRequestTelemetry>) -> Self {
        Self {
            telemetry,
            started_at: Instant::now(),
            sse_frame_count: 0,
            json_frame_count: 0,
            zero_runtime_event_frame_count: 0,
            consecutive_zero_runtime_event_frame_count: 0,
            emitted_runtime_event_count: 0,
            event_type_counts: HashMap::new(),
            last_event_type: None,
            tool_arg_lengths: HashMap::new(),
            tool_names: HashMap::new(),
            tool_call_ids: HashMap::new(),
        }
    }

    fn telemetry(&self) -> Option<&LlmRequestTelemetry> {
        self.telemetry.as_ref()
    }

    pub fn observe_sse_frame(&mut self, byte_len: usize) {
        self.sse_frame_count = self.sse_frame_count.saturating_add(1);
        let telemetry = self.telemetry();
        tracing::trace!(
            target: "den.llm.stream",
            api_style = "responses_stream",
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            sse_frame_count = self.sse_frame_count,
            frame_bytes = byte_len,
            elapsed_ms = self.started_at.elapsed().as_millis(),
            "LLM Responses stream SSE frame parsed"
        );
    }

    pub fn observe_json_frame(&mut self, json: &Value) {
        self.json_frame_count = self.json_frame_count.saturating_add(1);
        let event_type = json
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        *self
            .event_type_counts
            .entry(event_type.to_string())
            .or_default() += 1;
        self.last_event_type = Some(event_type.to_string());

        let key = response_item_key(json);
        if let Some(delta) = json.get("delta").and_then(Value::as_str) {
            if event_type == "response.function_call_arguments.delta" {
                let total = self.tool_arg_lengths.entry(key.clone()).or_insert(0);
                *total = total.saturating_add(delta.len());
            }
        }
        if let Some(arguments) = json.get("arguments").and_then(Value::as_str) {
            if event_type == "response.function_call_arguments.done" {
                self.tool_arg_lengths.insert(key.clone(), arguments.len());
            }
        }
        if let Some(item) = json.get("item") {
            let item_key = response_item_key_from_item(json, item);
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                if let Some(name) = item
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    self.tool_names.insert(item_key.clone(), name.to_string());
                }
                if let Some(call_id) = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    self.tool_call_ids
                        .insert(item_key.clone(), call_id.to_string());
                }
                if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                    self.tool_arg_lengths.insert(item_key, arguments.len());
                }
            }
        }

        let telemetry = self.telemetry();
        tracing::trace!(
            target: "den.llm.stream",
            api_style = "responses_stream",
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            json_frame_count = self.json_frame_count,
            event_type,
            elapsed_ms = self.started_at.elapsed().as_millis(),
            "LLM Responses stream JSON frame parsed"
        );
    }

    fn observe_response_parse(&mut self, event_type: &str, emitted_events: &[RuntimeStreamEvent]) {
        if emitted_events.is_empty() {
            self.zero_runtime_event_frame_count =
                self.zero_runtime_event_frame_count.saturating_add(1);
            self.consecutive_zero_runtime_event_frame_count = self
                .consecutive_zero_runtime_event_frame_count
                .saturating_add(1);
            let telemetry = self.telemetry();
            tracing::trace!(
                target: "den.llm.stream",
                api_style = "responses_stream",
                request_id = telemetry.and_then(|t| t.request_id.as_deref()),
                run_id = telemetry.and_then(|t| t.run_id.as_deref()),
                session_id = telemetry.and_then(|t| t.session_id.as_deref()),
                conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
                event_type,
                zero_runtime_event_frame_count = self.zero_runtime_event_frame_count,
                consecutive_zero_runtime_event_frame_count = self.consecutive_zero_runtime_event_frame_count,
                elapsed_ms = self.started_at.elapsed().as_millis(),
                "LLM Responses stream provider event emitted no runtime events"
            );
            return;
        }
        self.consecutive_zero_runtime_event_frame_count = 0;
        self.observe_emitted_events(emitted_events);
    }

    pub fn observe_emitted_events(&mut self, events: &[RuntimeStreamEvent]) {
        if events.is_empty() {
            return;
        }
        self.emitted_runtime_event_count = self
            .emitted_runtime_event_count
            .saturating_add(events.len() as u64);
        let telemetry = self.telemetry();
        let event_kinds = events.iter().map(runtime_event_kind).collect::<Vec<_>>();
        tracing::debug!(
            target: "den.llm.stream",
            api_style = "responses_stream",
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            emitted_runtime_event_count = self.emitted_runtime_event_count,
            emitted_this_frame = events.len(),
            event_kinds = ?event_kinds,
            zero_runtime_event_frame_count = self.zero_runtime_event_frame_count,
            consecutive_zero_runtime_event_frame_count = self.consecutive_zero_runtime_event_frame_count,
            last_provider_event_type = self.last_event_type.as_deref(),
            event_type_counts = ?self.event_type_counts,
            tool_arg_lengths = ?self.tool_arg_lengths,
            tool_names = ?self.tool_names,
            tool_call_ids = ?self.tool_call_ids,
            elapsed_ms = self.started_at.elapsed().as_millis(),
            "LLM Responses stream runtime events emitted"
        );
    }

    pub fn observe_incomplete_frame_on_end(&self, buffered_bytes: usize) {
        let telemetry = self.telemetry();
        tracing::warn!(
            target: "den.llm.stream",
            api_style = "responses_stream",
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            buffered_bytes,
            sse_frame_count = self.sse_frame_count,
            json_frame_count = self.json_frame_count,
            emitted_runtime_event_count = self.emitted_runtime_event_count,
            zero_runtime_event_frame_count = self.zero_runtime_event_frame_count,
            consecutive_zero_runtime_event_frame_count = self.consecutive_zero_runtime_event_frame_count,
            last_provider_event_type = self.last_event_type.as_deref(),
            event_type_counts = ?self.event_type_counts,
            tool_arg_lengths = ?self.tool_arg_lengths,
            tool_names = ?self.tool_names,
            tool_call_ids = ?self.tool_call_ids,
            elapsed_ms = self.started_at.elapsed().as_millis(),
            "LLM Responses stream ended with incomplete SSE frame"
        );
    }
}

impl ResponsesStreamAccumulator {
    pub fn ingest_sse_data_line(&mut self, json: &Value) -> OpenAiStreamParseResult {
        self.ingest_sse_data_line_with_diagnostics(json, None)
    }

    pub fn ingest_sse_data_line_with_diagnostics(
        &mut self,
        json: &Value,
        mut diagnostics: Option<&mut ResponsesStreamDiagnostics>,
    ) -> OpenAiStreamParseResult {
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.observe_json_frame(json);
        }
        let mut out = OpenAiStreamParseResult::default();
        if let Some(error) = json
            .get("error")
            .filter(|value| !value.is_null())
            .or_else(|| {
                json.pointer("/response/error")
                    .filter(|value| !value.is_null())
            })
        {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("LLM responses provider error")
                .to_string();
            out.events
                .push(RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error {
                    message,
                    detail: Some(error.to_string()),
                    error_type: error
                        .get("type")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    request_id: None,
                    context: None,
                }));
            self.completed = true;
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.observe_response_parse("error", &out.events);
            }
            return out;
        }
        let event_type = json.get("type").and_then(Value::as_str).unwrap_or_default();
        match event_type {
            "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(delta) = json
                    .get("delta")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    out.events.push(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::AssistantTextDelta {
                            text: delta.to_string(),
                        },
                    ));
                }
            }
            kind if kind.contains("reasoning") && kind.ends_with(".delta") => {
                if let Some(delta) = json
                    .get("delta")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    out.events.push(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::ReasoningTextDelta {
                            text: delta.to_string(),
                        },
                    ));
                }
            }
            "response.function_call_arguments.delta" => {
                let key = response_item_key(json);
                if let Some(delta) = json.get("delta").and_then(Value::as_str) {
                    self.tool_args.entry(key).or_default().push_str(delta);
                }
            }
            "response.function_call_arguments.done" => {
                let key = response_item_key(json);
                if let Some(arguments) = json.get("arguments").and_then(Value::as_str) {
                    self.tool_args.insert(key.clone(), arguments.to_string());
                }
                if let Some(event) = self.emit_tool_call_for_key(key) {
                    out.events.push(event);
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                if let Some(item) = json.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        self.saw_tool_call = true;
                        let key = response_item_key_from_item(json, item);
                        if let Some(name) = item
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            self.tool_names.insert(key.clone(), name.to_string());
                        }
                        if let Some(call_id) = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            self.tool_call_ids.insert(key.clone(), call_id.to_string());
                        }
                        if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                            self.tool_args.insert(key.clone(), arguments.to_string());
                        }
                        if event_type == "response.output_item.done" {
                            if let Some(event) = self.emit_tool_call_for_key(key) {
                                out.events.push(event);
                            }
                        }
                    }
                }
            }
            "response.completed" => {
                self.completed = true;
                out.finish_reason = Some("stop".to_string());
                if !self.saw_tool_call {
                    out.events.push(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::TurnCompleted { turn: None },
                    ));
                } else if let Some(event) = self.pending_tool_call_event_or_failure() {
                    out.events.push(event);
                }
            }
            "response.failed" | "response.incomplete" => {
                self.completed = true;
                let message = json
                    .pointer("/response/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Responses API turn failed")
                    .to_string();
                out.events.push(RuntimeStreamEvent::Semantic(
                    RuntimeSemanticEvent::TurnFailed {
                        turn: None,
                        category: RuntimeErrorCategory::BackendProtocol,
                        message,
                    },
                ));
            }
            _ => {}
        }
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.observe_response_parse(event_type, &out.events);
        }
        out
    }

    fn emit_tool_call_for_key(&mut self, key: String) -> Option<RuntimeStreamEvent> {
        if self.emitted_tool_call_keys.contains(&key) {
            return None;
        }
        let tool_name = self.tool_names.get(&key).cloned().unwrap_or_default();
        if tool_name.trim().is_empty() {
            return None;
        }
        self.emitted_tool_call_keys.insert(key.clone());
        let tool_call_id = self
            .tool_call_ids
            .get(&key)
            .cloned()
            .unwrap_or_else(|| key.clone());
        let arguments = self
            .tool_args
            .get(&key)
            .map(|raw| parse_tool_arguments(raw))
            .unwrap_or_else(|| Value::Object(Default::default()));
        Some(RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::ToolCallRequested {
                tool_call_id,
                tool_name,
                title: None,
                kind: Some("function".to_string()),
                arguments,
                approval_request_id: None,
                approval_required: false,
                approval_reason: None,
                run_id: None,
            },
        ))
    }

    fn pending_tool_call_event_or_failure(&mut self) -> Option<RuntimeStreamEvent> {
        let keys = self
            .tool_call_ids
            .keys()
            .chain(self.tool_names.keys())
            .chain(self.tool_args.keys())
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(event) = self.emit_tool_call_for_key(key) {
                return Some(event);
            }
        }
        if self.emitted_tool_call_keys.is_empty() {
            return Some(RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed {
                turn: None,
                category: RuntimeErrorCategory::BackendProtocol,
                message: "Responses stream completed after a function-call start without enough tool-call metadata to continue".to_string(),
            }));
        }
        None
    }

    pub fn should_detach_upstream(&self) -> bool {
        self.completed
    }

    pub fn flush_end_of_stream(&mut self) -> Vec<RuntimeStreamEvent> {
        if self.completed {
            Vec::new()
        } else if self.saw_tool_call {
            self.pending_tool_call_event_or_failure()
                .into_iter()
                .collect()
        } else {
            vec![RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::TurnCompleted { turn: None },
            )]
        }
    }
}

fn response_item_key(json: &Value) -> String {
    json.get("item_id")
        .or_else(|| json.get("output_index"))
        .or_else(|| json.get("item_index"))
        .and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        })
        .unwrap_or_else(|| "0".to_string())
}

fn response_item_key_from_item(json: &Value, item: &Value) -> String {
    item.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| response_item_key(json))
}

pub fn responses_sse_frame_to_runtime_events(
    accumulator: &mut ResponsesStreamAccumulator,
    body: &[u8],
) -> Result<Vec<RuntimeStreamEvent>, den_core::DenError> {
    responses_sse_frame_to_runtime_events_with_diagnostics(accumulator, body, None)
}

pub fn responses_sse_frame_to_runtime_events_with_diagnostics(
    accumulator: &mut ResponsesStreamAccumulator,
    body: &[u8],
    mut diagnostics: Option<&mut ResponsesStreamDiagnostics>,
) -> Result<Vec<RuntimeStreamEvent>, den_core::DenError> {
    if let Some(diagnostics) = diagnostics.as_deref_mut() {
        diagnostics.observe_sse_frame(body.len());
    }
    let text = std::str::from_utf8(body).map_err(|_| {
        den_core::DenError::System("invalid UTF-8 in LLM Responses SSE frame".to_string())
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
            let flushed = accumulator.flush_end_of_stream();
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.observe_emitted_events(&flushed);
            }
            events.extend(flushed);
            continue;
        }
        let json = serde_json::from_str::<Value>(data).map_err(|e| {
            den_core::DenError::System(format!("invalid LLM Responses SSE JSON: {e}"))
        })?;
        let parsed =
            accumulator.ingest_sse_data_line_with_diagnostics(&json, diagnostics.as_deref_mut());
        events.extend(parsed.events);
    }
    Ok(events)
}

pub fn openai_sse_chunk_to_runtime_events(
    chunk_body: &[u8],
) -> Result<Vec<RuntimeStreamEvent>, den_core::DenError> {
    let mut events = Vec::new();
    let text = std::str::from_utf8(chunk_body)
        .map_err(|_| den_core::DenError::System("invalid UTF-8 in LLM SSE chunk".to_string()))?;
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
        let json = serde_json::from_str::<Value>(data)
            .map_err(|e| den_core::DenError::System(format!("invalid LLM SSE JSON: {e}")))?;
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
    openai_sse_frame_to_runtime_events_with_diagnostics(accumulator, body, None)
}

pub fn openai_sse_frame_to_runtime_events_with_diagnostics(
    accumulator: &mut OpenAiStreamAccumulator,
    body: &[u8],
    mut diagnostics: Option<&mut OpenAiStreamDiagnostics>,
) -> Result<Vec<RuntimeStreamEvent>, den_core::DenError> {
    if let Some(diagnostics) = diagnostics.as_deref_mut() {
        diagnostics.observe_sse_frame(body.len());
    }
    let text = std::str::from_utf8(body)
        .map_err(|_| den_core::DenError::System("invalid UTF-8 in LLM SSE frame".to_string()))?;
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
            let flushed = accumulator.flush_end_of_stream();
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.observe_emitted_events(&flushed);
            }
            events.extend(flushed);
            continue;
        }
        let json = serde_json::from_str::<Value>(data)
            .map_err(|e| den_core::DenError::System(format!("invalid LLM SSE JSON: {e}")))?;
        let parsed =
            accumulator.ingest_sse_data_line_with_diagnostics(&json, diagnostics.as_deref_mut());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_stream_ignores_null_top_level_error() {
        let mut accumulator = ResponsesStreamAccumulator::default();
        let parsed = accumulator.ingest_sse_data_line(&serde_json::json!({
            "type": "response.created",
            "error": null,
            "response": {
                "id": "resp_1",
                "error": null,
            }
        }));

        assert!(parsed.events.is_empty());
        assert!(!accumulator.should_detach_upstream());
    }

    #[test]
    fn responses_stream_ignores_null_nested_response_error() {
        let mut accumulator = ResponsesStreamAccumulator::default();
        let parsed = accumulator.ingest_sse_data_line(&serde_json::json!({
            "type": "response.in_progress",
            "response": {
                "id": "resp_1",
                "error": null,
            }
        }));

        assert!(parsed.events.is_empty());
        assert!(!accumulator.should_detach_upstream());
    }

    #[test]
    fn responses_stream_maps_non_null_error() {
        let mut accumulator = ResponsesStreamAccumulator::default();
        let parsed = accumulator.ingest_sse_data_line(&serde_json::json!({
            "type": "response.created",
            "response": {
                "id": "resp_1",
                "error": {
                    "type": "invalid_request_error",
                    "message": "bad request"
                }
            }
        }));

        assert!(accumulator.should_detach_upstream());
        assert!(matches!(
            parsed.events.as_slice(),
            [RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error {
                message,
                error_type: Some(error_type),
                ..
            })] if message == "bad request" && error_type == "invalid_request_error"
        ));
    }
}
