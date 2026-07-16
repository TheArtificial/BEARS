use den_service::bears::prompt_fragments::{
    render_turn_fragment, repository_prompt_fragment_registry,
};
use serde_json::{json, Value};

const LOG_SAMPLE_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOperationalOutcomeProjection {
    pub model_summary: String,
    pub content: Value,
    pub user_message: Option<String>,
    pub history_marker: Option<String>,
    pub diagnostic_context: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHistoryMarkerProjection {
    pub kind: String,
    pub text: String,
    pub metadata: Value,
}

pub fn log_sample(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    let mut out = value.chars().take(LOG_SAMPLE_CHARS).collect::<String>();
    if value.chars().count() > LOG_SAMPLE_CHARS {
        out.push('…');
    }
    out
}

pub fn run_failure_projection(
    reason: &str,
    message: &str,
    run_id: &str,
    bear_name: &str,
    context: Option<Value>,
) -> RuntimeOperationalOutcomeProjection {
    let diagnostic_context = failure_context_with_diagnostics(reason, message, context);
    let (model_summary, content) =
        normalized_operational_outcome(reason, message, run_id, diagnostic_context.as_ref());
    RuntimeOperationalOutcomeProjection {
        model_summary,
        content,
        user_message: run_failed_user_message(reason, message, bear_name),
        history_marker: run_failed_history_marker(reason, message, bear_name),
        diagnostic_context,
    }
}

pub fn normalized_operational_outcome(
    reason: &str,
    message: &str,
    run_id: &str,
    context: Option<&Value>,
) -> (String, Value) {
    let autonomous_resume = context
        .and_then(|value| value.get("autonomous_resume_obligation"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (kind, retryable, subsystem) = match reason {
        "continuation_stream_error" => ("provider_stream_error", true, "llm_stream_transport"),
        "continuation_watchdog_timeout" => ("continuation_timeout", true, "continuation_runtime"),
        "continuation_start_failed" => ("continuation_start_failed", true, "continuation_runtime"),
        "runtime_internal" if is_budget_or_loop_failure(reason, message) => {
            ("turn_budget_exhausted", false, "turn_budget")
        }
        _ => ("operational_failure", false, "runtime"),
    };
    let rendered_summary = render_operational_outcome_summary(kind);
    let summary = autonomous_resume.unwrap_or(&rendered_summary);
    let mut content = json!({
            "source": "den.runtime",
            "event": "operational_outcome",
            "scope_id": run_id,
            "request_id": run_id,
            "kind": kind,
            "reason": reason,
            "retryable": retryable,
            "subsystem": subsystem,
            "run_id": run_id,
            "summary": summary,
            "detail": log_sample(message),
    });
    if let Some(context) = context {
        content["context"] = context.clone();
    }
    (summary.to_string(), content)
}

fn render_operational_outcome_summary(kind: &str) -> String {
    render_operational_outcome_summary_from_fragments(kind).unwrap_or_else(|_| {
        // ponytail: fallback avoids losing runtime error reporting if fragment loading fails;
        // upgrade path is to plumb DenError through normalized_operational_outcome callers.
        "Previous turn ended before final answer delivery. Verify persisted state, recent tool results, and any task/worktree status before deciding whether there is work left to do."
            .to_string()
    })
}

fn render_operational_outcome_summary_from_fragments(
    kind: &str,
) -> Result<String, den_core::DenError> {
    let fragments = repository_prompt_fragment_registry()?;
    let fragment = fragments.require("runtime_operational_outcome_summary")?;
    render_turn_fragment(fragment, &json!({ "outcome": { "kind": kind } }))
        .map(|text| text.trim().to_string())
}

pub fn is_budget_or_loop_failure(reason: &str, message: &str) -> bool {
    reason == "runtime_internal" && (message.contains("budget") || message.contains("rule of ko"))
}

pub fn run_failed_user_message(reason: &str, message: &str, bear_name: &str) -> Option<String> {
    if is_budget_or_loop_failure(reason, message) {
        return Some(format!(
            "{} stopped this turn after it ran too long. Recent tool results were preserved, but no final answer was delivered. Start a fresh turn to continue safely.",
            display_bear_name(bear_name)
        ));
    }
    None
}

pub fn run_failed_history_marker(reason: &str, message: &str, bear_name: &str) -> Option<String> {
    run_failed_user_message(reason, message, bear_name)
}

pub fn numeric_message_field(message: &str, field: &str) -> Option<u64> {
    let start = message.find(field)? + field.len();
    let rest = &message[start..];
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty())
        .then(|| digits.parse::<u64>().ok())
        .flatten()
}

pub fn failure_context_with_diagnostics(
    reason: &str,
    message: &str,
    context: Option<Value>,
) -> Option<Value> {
    let mut context = context.unwrap_or_else(|| json!({}));
    let Some(object) = context.as_object_mut() else {
        return Some(json!({
            "diagnostic": {
                "reason": reason,
                "raw_message": message,
            },
            "previous_context": context,
        }));
    };
    let mut diagnostic = json!({
        "reason": reason,
        "raw_message": message,
    });
    if is_budget_or_loop_failure(reason, message) {
        diagnostic["class"] = json!("turn_budget_exhausted");
        diagnostic["model_action"] = json!("none");
        if let Some(elapsed_ms) = numeric_message_field(message, "elapsed=") {
            diagnostic["elapsed_ms"] = json!(elapsed_ms);
        }
        if let Some(limit_ms) = numeric_message_field(message, "limit=") {
            diagnostic["limit_ms"] = json!(limit_ms);
        }
    }
    object.insert("diagnostic".to_string(), diagnostic);
    Some(context)
}

pub fn runtime_progress_history_marker(
    bear_name: &str,
    kind: &str,
    text: Option<&str>,
    detail: Option<&Value>,
) -> Option<RuntimeHistoryMarkerProjection> {
    match kind {
        "turn_budget_warning" => Some(RuntimeHistoryMarkerProjection {
            kind: kind.to_string(),
            text: budget_warning_history_marker(bear_name, detail),
            metadata: json!({ "kind": kind, "runtime_text": text, "detail": detail }),
        }),
        "autonomous_continuation_gate" => {
            let next_task = detail
                .and_then(|value| value.get("next_task"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let text = if let Some(next_task) = next_task {
                format!(
                    "{} was kept on the current task focus and asked to continue `{next_task}`.",
                    display_bear_name(bear_name)
                )
            } else {
                format!(
                    "{} was kept on the current task focus and asked to continue the next incomplete item.",
                    display_bear_name(bear_name)
                )
            };
            Some(RuntimeHistoryMarkerProjection {
                kind: kind.to_string(),
                metadata: json!({ "kind": kind, "runtime_text": text, "detail": detail }),
                text,
            })
        }
        _ => None,
    }
}

pub fn runtime_event_history_marker(
    bear_name: &str,
    event: &den_protocol::RuntimeStreamEvent,
) -> Option<RuntimeHistoryMarkerProjection> {
    match event {
        den_protocol::RuntimeStreamEvent::Semantic(
            den_protocol::RuntimeSemanticEvent::RunProgress {
                kind, text, detail, ..
            },
        ) => runtime_progress_history_marker(bear_name, kind, text.as_deref(), detail.as_ref()),
        _ => None,
    }
}

fn budget_warning_history_marker(bear_name: &str, detail: Option<&Value>) -> String {
    let budget_kind = detail
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str)
        .map(budget_kind_from_code)
        .unwrap_or("runtime budget");
    format!(
        "{} was warned about the number of {budget_kind} for this turn.",
        display_bear_name(bear_name)
    )
}

fn budget_kind_from_code(code: &str) -> &'static str {
    match code {
        "tool_budget_finalization_warning" => "tool calls",
        "emergency_hard_step_warning" => "continuation steps",
        "wall_clock_warning" => "wall-clock time",
        "total_tool_budget_warning" => "tool calls",
        "tool_class_budget_warning" => "tool calls in one tool class",
        "failure_budget_warning" => "failed tool batches",
        "rule_of_ko_warning" => "repeated tool batches",
        _ => "runtime budget",
    }
}

fn display_bear_name(bear_name: &str) -> &str {
    let trimmed = bear_name.trim();
    if trimmed.is_empty() {
        "The bear"
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_failure_has_friendly_user_message_and_diagnostics() {
        let message = "I stopped because this turn exhausted its wall-clock budget \
            (elapsed=252985ms/limit=240000ms).";

        let projection =
            run_failure_projection("runtime_internal", message, "run-1", "Builder Bear", None);
        assert_eq!(
            projection.user_message.as_deref(),
            Some("Builder Bear stopped this turn after it ran too long. Recent tool results were preserved, but no final answer was delivered. Start a fresh turn to continue safely.")
        );
        assert_eq!(projection.content["source"], "den.runtime");

        let context = projection.diagnostic_context.expect("diagnostic context");
        assert_eq!(context["diagnostic"]["class"], "turn_budget_exhausted");
        assert_eq!(context["diagnostic"]["model_action"], "none");
        assert_eq!(context["diagnostic"]["elapsed_ms"], 252985);
        assert_eq!(context["diagnostic"]["limit_ms"], 240000);
    }

    #[test]
    fn budget_operational_outcome_tells_model_no_repair_action() {
        let projection = run_failure_projection(
            "runtime_internal",
            "I stopped because this turn exhausted its wall-clock budget \
                (elapsed=252985ms/limit=240000ms).",
            "run-1",
            "Builder Bear",
            None,
        );

        assert!(
            projection
                .model_summary
                .contains("no infrastructure repair action"),
            "{}",
            projection.model_summary
        );
        assert!(!projection
            .model_summary
            .contains("Operational note from Den"));
        assert_eq!(projection.content["kind"], "turn_budget_exhausted");
        assert_eq!(projection.content["retryable"], false);
    }

    #[test]
    fn generic_operational_outcome_tells_model_to_verify_persisted_state() {
        let projection = run_failure_projection(
            "runtime_internal_unknown",
            "unexpected runtime failure",
            "run-1",
            "Builder Bear",
            None,
        );

        assert!(
            projection.model_summary.contains("Verify persisted state"),
            "{}",
            projection.model_summary
        );
        assert!(
            projection
                .model_summary
                .contains("before deciding whether there is work left to do"),
            "{}",
            projection.model_summary
        );
        assert!(!projection
            .model_summary
            .contains("continue from the latest successful state"));
        assert_eq!(projection.content["kind"], "operational_failure");
        assert_eq!(projection.content["retryable"], false);
    }

    #[test]
    fn runtime_progress_history_marker_covers_budget_and_task_focus() {
        let budget_marker = runtime_progress_history_marker(
            "Builder Bear",
            "turn_budget_warning",
            Some("Budget advisory: next read will stop the turn."),
            Some(&json!({ "code": "tool_class_budget_warning" })),
        )
        .expect("budget marker");
        assert_eq!(
            budget_marker.text,
            "Builder Bear was warned about the number of tool calls in one tool class for this turn."
        );
        assert_eq!(budget_marker.metadata["kind"], "turn_budget_warning");

        let task_marker = runtime_progress_history_marker(
            "Builder Bear",
            "autonomous_continuation_gate",
            None,
            Some(&json!({ "next_task": "Patch runtime markers" })),
        )
        .expect("task focus marker");
        assert!(task_marker.text.contains("Patch runtime markers"));
        assert_eq!(task_marker.metadata["kind"], "autonomous_continuation_gate");
    }
}
