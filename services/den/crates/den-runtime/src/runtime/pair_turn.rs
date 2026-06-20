use serde_json::Value;

/// Shared boundary for sending a `pair` role turn to a runtime provider.
///
/// Invariant: `human_message` is the only user-authored content serialized into model input.
/// Channel/runtime context must flow through structured fields (`client_tools`, Den tool context,
/// session_info backing state, UI events, or explicit `override_system` replacement semantics), not
/// by concatenating text into the user message.
pub struct PairTurnRequest<'a> {
    pub conversation_id: &'a str,
    pub binding_id: &'a str,
    pub human_message: &'a str,
    pub client_tools: Option<Value>,
    pub stream_tokens: bool,
    pub override_system: Option<&'a str>,
    pub boundary: PairTurnBoundaryLog<'a>,
}

pub struct PairTurnBoundaryLog<'a> {
    pub request_id: &'a str,
    pub channel_family: &'a str,
    pub session_id: &'a str,
    pub runtime_context_len: usize,
}

impl PairTurnRequest<'_> {
    pub fn client_tools_count(&self) -> usize {
        self.client_tools
            .as_ref()
            .and_then(|tools| tools.as_array())
            .map(Vec::len)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pair_turn_request_counts_client_tools() {
        let request = PairTurnRequest {
            conversation_id: "conv-test",
            binding_id: "agent-test",
            human_message: "hello",
            client_tools: Some(json!([{ "name": "session_info" }, { "name": "memory_read" }])),
            stream_tokens: false,
            override_system: None,
            boundary: PairTurnBoundaryLog {
                request_id: "request-test",
                channel_family: "acp",
                session_id: "session-test",
                runtime_context_len: 42,
            },
        };
        assert_eq!(request.client_tools_count(), 2);
    }
}
