use den_runtime::{
    gateway_events::GatewayEvent,
    agent_assist::normalize_display_status_text,
};

use super::text_utils::{
    acp_max_thought_bytes_per_turn, should_flush_text, truncate_utf8_boundary,
};

#[derive(Default)]
pub(in crate::acp) struct AcpTextChunker {
    assistant: String,
    reasoning: String,
    max_chars: usize,
    max_reasoning_bytes: usize,
    emitted_reasoning_bytes: usize,
    reasoning_limit_reached: bool,
}

impl AcpTextChunker {
    pub(in crate::acp) fn new(max_chars: usize) -> Self {
        Self::new_with_reasoning_limit(max_chars, acp_max_thought_bytes_per_turn())
    }

    pub(in crate::acp) fn new_with_reasoning_limit(max_chars: usize, max_reasoning_bytes: usize) -> Self {
        Self {
            assistant: String::new(),
            reasoning: String::new(),
            max_chars,
            max_reasoning_bytes,
            emitted_reasoning_bytes: 0,
            reasoning_limit_reached: false,
        }
    }

    pub(in crate::acp) fn push(&mut self, event: GatewayEvent) -> Vec<GatewayEvent> {
        match event {
            GatewayEvent::AssistantTextDelta { text } => {
                self.assistant.push_str(&text);
                if should_flush_text(&self.assistant, self.max_chars) {
                    self.flush_assistant().into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            GatewayEvent::StatusText { text } => {
                if self.reasoning_limit_reached {
                    return Vec::new();
                }
                let first_status_for_turn =
                    self.emitted_reasoning_bytes == 0 && self.reasoning.is_empty();
                self.reasoning.push_str(&text);
                if first_status_for_turn || should_flush_text(&self.reasoning, self.max_chars) {
                    self.flush_reasoning().into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            other => {
                let mut events = self.flush_all();
                events.push(other);
                events
            }
        }
    }

    pub(in crate::acp) fn flush_assistant(&mut self) -> Option<GatewayEvent> {
        if self.assistant.is_empty() {
            None
        } else {
            Some(GatewayEvent::AssistantTextDelta {
                text: std::mem::take(&mut self.assistant),
            })
        }
    }

    pub(in crate::acp) fn flush_reasoning(&mut self) -> Option<GatewayEvent> {
        if self.reasoning.is_empty() || self.reasoning_limit_reached {
            self.reasoning.clear();
            return None;
        }
        let remaining = self
            .max_reasoning_bytes
            .saturating_sub(self.emitted_reasoning_bytes);
        if remaining == 0 {
            self.reasoning.clear();
            self.reasoning_limit_reached = true;
            return Some(GatewayEvent::StatusText {
                text: normalize_display_status_text(
                    "BEARS suppressed additional thinking/status output for this turn because it exceeded the safety limit",
                ),
            });
        }

        let mut text = std::mem::take(&mut self.reasoning);
        if text.len() > remaining {
            text = truncate_utf8_boundary(&text, remaining).to_string();
            self.reasoning_limit_reached = true;
            text.push('\n');
            text.push_str(&normalize_display_status_text(
                "BEARS suppressed additional thinking/status output for this turn because it exceeded the safety limit",
            ));
        }
        self.emitted_reasoning_bytes = self.emitted_reasoning_bytes.saturating_add(text.len());
        Some(GatewayEvent::StatusText { text })
    }

    pub(in crate::acp) fn flush_all(&mut self) -> Vec<GatewayEvent> {
        self.flush_assistant()
            .into_iter()
            .chain(self.flush_reasoning())
            .collect()
    }
}

