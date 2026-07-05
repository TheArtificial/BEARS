use den_llm::{model_registry, ChatCompletionRequest};
use den_protocol::{
    ContextBudgetComponentReport, ContextBudgetEstimatePrecision, ContextBudgetReport,
};

#[derive(Debug, Clone, Default)]
pub struct AssembledTurnBudgetComponents {
    pub compiled_prompt_chars: u32,
    pub key_memory_projection_chars: u32,
    pub recall_chars: u32,
    pub runtime_supplement_chars: u32,
    pub tool_surface_guidance_chars: u32,
    pub compaction_chars: u32,
    pub transcript_chars: u32,
    pub transcript_fallback_pruned_chars: u32,
    pub transcript_fallback_pruned_messages: u32,
    pub current_user_input_chars: u32,
    pub tool_message_chars: u32,
}

fn estimated_tokens(chars: u32) -> u32 {
    chars.saturating_add(3) / 4
}

fn component(key: &str, label: &str, chars: u32) -> ContextBudgetComponentReport {
    component_with_label(key, label.to_string(), chars)
}

fn component_with_label(key: &str, label: String, chars: u32) -> ContextBudgetComponentReport {
    ContextBudgetComponentReport {
        key: key.to_string(),
        label,
        estimated_tokens: estimated_tokens(chars),
        estimated_characters: chars,
    }
}

pub fn estimate_context_budget(
    request: &ChatCompletionRequest,
    parts: &AssembledTurnBudgetComponents,
    fallback_context_window: Option<u32>,
    fallback_max_output_tokens: Option<u32>,
) -> ContextBudgetReport {
    let body = request.to_body().to_string();
    let body_chars = body.chars().count() as u32;
    let tool_schema_chars = serde_json::to_string(&request.tools)
        .map(|value| value.chars().count() as u32)
        .unwrap_or_default();
    let mut components = vec![
        component(
            "compiled_prompt",
            "Compiled prompt",
            parts.compiled_prompt_chars,
        ),
        component(
            "key_memory_projection",
            "Key memory projection",
            parts.key_memory_projection_chars,
        ),
        component("recall", "Retrieved recall", parts.recall_chars),
        component(
            "runtime_supplement",
            "Runtime supplement",
            parts.runtime_supplement_chars,
        ),
        component(
            "tool_surface_guidance",
            "Tool surface guidance",
            parts.tool_surface_guidance_chars,
        ),
        component("compaction", "Compaction context", parts.compaction_chars),
        component("transcript", "Transcript replay", parts.transcript_chars),
        component(
            "current_user_input",
            "Current user input",
            parts.current_user_input_chars,
        ),
        component("tool_messages", "Tool messages", parts.tool_message_chars),
        component("tool_schemas", "Tool schemas", tool_schema_chars),
    ];
    if parts.transcript_fallback_pruned_chars > 0 {
        components.push(component_with_label(
            "transcript_fallback_prune",
            format!(
                "Transcript fallback prune ({} messages)",
                parts.transcript_fallback_pruned_messages
            ),
            parts.transcript_fallback_pruned_chars,
        ));
    }
    let accounted_chars: u32 = components
        .iter()
        .map(|entry| entry.estimated_characters)
        .sum();
    if body_chars > accounted_chars {
        components.push(component(
            "request_overhead",
            "Request framing overhead",
            body_chars - accounted_chars,
        ));
    }

    let total_input_tokens = estimated_tokens(body_chars);
    let model_entry = model_registry::entry_for_handle(&request.model);
    let context_window = model_entry
        .map(|entry| entry.context_window)
        .or(fallback_context_window);
    let max_output_tokens = model_entry
        .and_then(|entry| entry.max_output_tokens)
        .or(fallback_max_output_tokens);
    let reserved_output_tokens = request
        .max_tokens
        .unwrap_or_else(|| max_output_tokens.unwrap_or(2048))
        .min(max_output_tokens.unwrap_or(u32::MAX))
        .min(4096);
    let estimated_total_tokens = total_input_tokens.saturating_add(reserved_output_tokens);
    let near_budget = context_window.is_some_and(|limit| {
        estimated_total_tokens.saturating_mul(100) >= limit.saturating_mul(90)
    });
    let over_budget = context_window.is_some_and(|limit| estimated_total_tokens > limit);

    ContextBudgetReport {
        model: request.model.clone(),
        context_window,
        max_output_tokens,
        reserved_output_tokens,
        estimated_input_tokens: total_input_tokens,
        estimated_total_tokens,
        estimate_precision: ContextBudgetEstimatePrecision::Approximate,
        near_budget,
        over_budget,
        components,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use den_llm::{ChatMessage, LlmToolDefinition};
    use serde_json::json;

    #[test]
    fn estimate_context_budget_reports_component_breakdown() {
        let request = ChatCompletionRequest {
            model: "openai/gpt-4.1".to_string(),
            messages: vec![ChatMessage {
                role: "system".to_string(),
                content: Some("system text".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            tools: vec![LlmToolDefinition {
                name: "session_info".to_string(),
                description: Some("Environment snapshot".to_string()),
                parameters: json!({"type":"object"}),
            }],
            stream: true,
            tool_choice: None,
            temperature: None,
            max_tokens: Some(512),
            telemetry: None,
        };
        let report = estimate_context_budget(
            &request,
            &AssembledTurnBudgetComponents {
                compiled_prompt_chars: 120,
                key_memory_projection_chars: 80,
                recall_chars: 40,
                runtime_supplement_chars: 20,
                tool_surface_guidance_chars: 60,
                compaction_chars: 10,
                transcript_chars: 200,
                transcript_fallback_pruned_chars: 24,
                transcript_fallback_pruned_messages: 3,
                current_user_input_chars: 32,
                tool_message_chars: 16,
            },
            None,
            None,
        );

        assert_eq!(report.model, "openai/gpt-4.1");
        assert_eq!(report.context_window, Some(1_047_576));
        assert_eq!(report.max_output_tokens, Some(32_768));
        assert_eq!(report.reserved_output_tokens, 512);
        assert_eq!(
            report.estimate_precision,
            ContextBudgetEstimatePrecision::Approximate
        );
        assert!(report.components.iter().any(|c| c.key == "compiled_prompt"));
        assert!(report.components.iter().any(|c| c.key == "tool_schemas"));
        assert!(report
            .components
            .iter()
            .any(|c| c.key == "transcript_fallback_prune" && c.label.contains("3 messages")));
        assert!(!report.over_budget);
    }

    #[test]
    fn estimate_context_budget_flags_over_budget() {
        let request = ChatCompletionRequest {
            model: "openai/gpt-4.1".to_string(),
            messages: vec![ChatMessage {
                role: "system".to_string(),
                content: Some("x".repeat(5_000_000)),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            tools: Vec::new(),
            stream: true,
            tool_choice: None,
            temperature: None,
            max_tokens: Some(4096),
            telemetry: None,
        };
        let report = estimate_context_budget(
            &request,
            &AssembledTurnBudgetComponents {
                compiled_prompt_chars: 5_000_000,
                ..Default::default()
            },
            None,
            None,
        );

        assert!(report.near_budget);
        assert!(report.over_budget);
    }
}
