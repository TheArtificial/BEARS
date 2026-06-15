use crate::llm::{ChatMessage, ChatToolCall};

/// Returns tool calls from the latest assistant message that do not yet have tool results.
pub fn pending_tool_calls(messages: &[ChatMessage]) -> Vec<ChatToolCall> {
    let mut open = Vec::<ChatToolCall>::new();
    for msg in messages {
        if msg.role == "assistant" {
            if let Some(calls) = &msg.tool_calls {
                if !calls.is_empty() {
                    open.clone_from(calls);
                }
            }
            continue;
        }
        if msg.role == "tool" {
            if let Some(id) = &msg.tool_call_id {
                open.retain(|call| call.id != *id);
            }
        }
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatToolCallFunction;

    fn call(id: &str) -> ChatToolCall {
        ChatToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: ChatToolCallFunction {
                name: "session_info".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    #[test]
    fn pending_tool_calls_returns_unanswered_assistant_tool_calls() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![call("call_1"), call("call_2")]),
            },
            ChatMessage {
                role: "tool".to_string(),
                content: Some("ok".to_string()),
                tool_call_id: Some("call_1".to_string()),
                name: None,
                tool_calls: None,
            },
        ];
        let pending = pending_tool_calls(&messages);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "call_2");
    }
}
