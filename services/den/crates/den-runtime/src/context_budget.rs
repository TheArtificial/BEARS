use den_llm::{
    model_registry::{self, ModelTokenCalibration},
    ChatCompletionRequest,
};
use den_protocol::{
    ContextBudgetCalibrationReport, ContextBudgetComponentReport, ContextBudgetEstimatePrecision,
    ContextBudgetReport,
};

#[derive(Debug, Clone, Default)]
pub struct AssembledTurnBudgetComponents {
    pub compiled_prompt_chars: u32,
    pub key_memory_projection_chars: u32,
    pub recall_chars: u32,
    pub runtime_supplement_chars: u32,
    pub capability_discovery_chars: u32,
    pub recently_discovered_capabilities_chars: u32,
    pub tool_surface_guidance_chars: u32,
    pub compaction_chars: u32,
    pub transcript_chars: u32,
    pub transcript_fallback_pruned_chars: u32,
    pub transcript_fallback_pruned_messages: u32,
    pub current_user_input_chars: u32,
    pub tool_message_chars: u32,
}

/// Tokens per million characters implied by the uncalibrated chars/4 heuristic.
const DEFAULT_TOKENS_PER_MILLION_CHARS: u32 = 250_000;

fn estimated_tokens(chars: u32, tokens_per_char: Option<f64>) -> u32 {
    match tokens_per_char {
        Some(ratio) => (f64::from(chars) * ratio).ceil() as u32,
        None => chars.saturating_add(3) / 4,
    }
}

fn component(
    key: &str,
    label: &str,
    chars: u32,
    tokens_per_char: Option<f64>,
) -> ContextBudgetComponentReport {
    component_with_label(key, label.to_string(), chars, tokens_per_char)
}

fn component_with_label(
    key: &str,
    label: String,
    chars: u32,
    tokens_per_char: Option<f64>,
) -> ContextBudgetComponentReport {
    ContextBudgetComponentReport {
        key: key.to_string(),
        label,
        estimated_tokens: estimated_tokens(chars, tokens_per_char),
        estimated_characters: chars,
    }
}

fn calibration_report(
    applied_tokens_per_char: Option<f64>,
    calibration: Option<ModelTokenCalibration>,
) -> ContextBudgetCalibrationReport {
    applied_tokens_per_char.map_or_else(
        || ContextBudgetCalibrationReport {
            source: "default".to_string(),
            tokens_per_million_chars: DEFAULT_TOKENS_PER_MILLION_CHARS,
            sample_count: 0,
        },
        |ratio| ContextBudgetCalibrationReport {
            source: "model_registry".to_string(),
            tokens_per_million_chars: (ratio * 1_000_000.0).round() as u32,
            sample_count: calibration.map_or(0, |calibration| calibration.sample_count),
        },
    )
}

pub fn estimate_context_budget(
    request: &ChatCompletionRequest,
    parts: &AssembledTurnBudgetComponents,
    fallback_context_window: Option<u32>,
    fallback_max_output_tokens: Option<u32>,
    calibration: Option<ModelTokenCalibration>,
) -> ContextBudgetReport {
    let tokens_per_char = calibration.and_then(ModelTokenCalibration::applied_tokens_per_char);
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
            tokens_per_char,
        ),
        component(
            "key_memory_projection",
            "Key memory projection",
            parts.key_memory_projection_chars,
            tokens_per_char,
        ),
        component(
            "recall",
            "Retrieved recall",
            parts.recall_chars,
            tokens_per_char,
        ),
        component(
            "runtime_supplement",
            "Runtime supplement",
            parts.runtime_supplement_chars,
            tokens_per_char,
        ),
        component(
            "capability_discovery",
            "Capability discovery guidance",
            parts.capability_discovery_chars,
            tokens_per_char,
        ),
        component(
            "recently_discovered_capabilities",
            "Recently discovered capabilities",
            parts.recently_discovered_capabilities_chars,
            tokens_per_char,
        ),
        component(
            "tool_surface_guidance",
            "Tool surface guidance",
            parts.tool_surface_guidance_chars,
            tokens_per_char,
        ),
        component(
            "compaction",
            "Compaction context",
            parts.compaction_chars,
            tokens_per_char,
        ),
        component(
            "transcript",
            "Transcript replay",
            parts.transcript_chars,
            tokens_per_char,
        ),
        component(
            "current_user_input",
            "Current user input",
            parts.current_user_input_chars,
            tokens_per_char,
        ),
        component(
            "tool_messages",
            "Tool messages",
            parts.tool_message_chars,
            tokens_per_char,
        ),
        component(
            "tool_schemas",
            "Tool schemas",
            tool_schema_chars,
            tokens_per_char,
        ),
    ];
    if parts.transcript_fallback_pruned_chars > 0 {
        components.push(component_with_label(
            "transcript_fallback_prune",
            format!(
                "Transcript fallback prune ({} messages)",
                parts.transcript_fallback_pruned_messages
            ),
            parts.transcript_fallback_pruned_chars,
            tokens_per_char,
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
            tokens_per_char,
        ));
    }

    let total_input_tokens = estimated_tokens(body_chars, tokens_per_char);
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
        estimate_precision: if tokens_per_char.is_some() {
            ContextBudgetEstimatePrecision::CalibratedApproximate
        } else {
            ContextBudgetEstimatePrecision::Approximate
        },
        near_budget,
        over_budget,
        calibration: Some(calibration_report(tokens_per_char, calibration)),
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
            thinking_effort: None,
            telemetry: None,
        };
        let report = estimate_context_budget(
            &request,
            &AssembledTurnBudgetComponents {
                compiled_prompt_chars: 120,
                key_memory_projection_chars: 80,
                recall_chars: 40,
                runtime_supplement_chars: 20,
                capability_discovery_chars: 12,
                recently_discovered_capabilities_chars: 18,
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
        let calibration = report.calibration.as_ref().expect("calibration provenance");
        assert_eq!(calibration.source, "default");
        assert_eq!(calibration.tokens_per_million_chars, 250_000);
        assert_eq!(calibration.sample_count, 0);
        assert!(report.components.iter().any(|c| c.key == "compiled_prompt"));
        assert!(report
            .components
            .iter()
            .any(|c| c.key == "capability_discovery" && c.estimated_characters == 12));
        assert!(report
            .components
            .iter()
            .any(|c| c.key == "recently_discovered_capabilities" && c.estimated_characters == 18));
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
            thinking_effort: None,
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
            None,
        );

        assert!(report.near_budget);
        assert!(report.over_budget);
    }

    fn calibration_request(prompt_chars: usize) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "openai/gpt-4.1".to_string(),
            messages: vec![ChatMessage {
                role: "system".to_string(),
                content: Some("y".repeat(prompt_chars)),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            tools: Vec::new(),
            stream: true,
            tool_choice: None,
            temperature: None,
            max_tokens: Some(512),
            thinking_effort: None,
            telemetry: None,
        }
    }

    #[test]
    fn estimate_context_budget_applies_calibrated_ratio() {
        let request = calibration_request(10_000);
        let calibration = den_llm::model_registry::ModelTokenCalibration {
            tokens_per_char: 0.5,
            sample_count: 10,
        };
        let report = estimate_context_budget(
            &request,
            &AssembledTurnBudgetComponents {
                compiled_prompt_chars: 10_000,
                ..Default::default()
            },
            None,
            None,
            Some(calibration),
        );

        assert_eq!(
            report.estimate_precision,
            ContextBudgetEstimatePrecision::CalibratedApproximate
        );
        let provenance = report.calibration.as_ref().expect("calibration provenance");
        assert_eq!(provenance.source, "model_registry");
        assert_eq!(provenance.tokens_per_million_chars, 500_000);
        assert_eq!(provenance.sample_count, 10);
        let compiled = report
            .components
            .iter()
            .find(|c| c.key == "compiled_prompt")
            .expect("compiled prompt component");
        assert_eq!(compiled.estimated_tokens, 5_000);
        // Total input estimate is roughly double the chars/4 heuristic.
        let uncalibrated = estimate_context_budget(
            &request,
            &AssembledTurnBudgetComponents {
                compiled_prompt_chars: 10_000,
                ..Default::default()
            },
            None,
            None,
            None,
        );
        assert!(report.estimated_input_tokens > uncalibrated.estimated_input_tokens);
    }

    #[test]
    fn estimate_context_budget_ignores_calibration_below_sample_threshold() {
        let request = calibration_request(10_000);
        let calibration = den_llm::model_registry::ModelTokenCalibration {
            tokens_per_char: 0.5,
            sample_count: den_llm::model_registry::MODEL_TOKEN_CALIBRATION_MIN_SAMPLES - 1,
        };
        let report = estimate_context_budget(
            &request,
            &AssembledTurnBudgetComponents {
                compiled_prompt_chars: 10_000,
                ..Default::default()
            },
            None,
            None,
            Some(calibration),
        );

        assert_eq!(
            report.estimate_precision,
            ContextBudgetEstimatePrecision::Approximate
        );
        let provenance = report.calibration.as_ref().expect("calibration provenance");
        assert_eq!(provenance.source, "default");
        assert_eq!(provenance.tokens_per_million_chars, 250_000);
    }
}
