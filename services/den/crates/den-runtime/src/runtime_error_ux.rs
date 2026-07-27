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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeIssueSeverity {
    Info,
    Warning,
    Recoverable,
    UserActionRequired,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeIssueDisposition {
    LogOnly,
    SurfaceDiagnostic,
    SteerModelAndContinue,
    AskUserAndPause,
    AbortRun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIssuePolicy {
    pub severity: RuntimeIssueSeverity,
    pub disposition: RuntimeIssueDisposition,
    pub code: &'static str,
    pub side_effects_blocked: bool,
    pub required_next_tool: Option<&'static str>,
}

impl RuntimeIssuePolicy {
    pub const fn soft_warning(code: &'static str) -> Self {
        Self {
            severity: RuntimeIssueSeverity::Warning,
            disposition: RuntimeIssueDisposition::SurfaceDiagnostic,
            code,
            side_effects_blocked: true,
            required_next_tool: None,
        }
    }

    pub const fn user_action_required(code: &'static str) -> Self {
        Self {
            severity: RuntimeIssueSeverity::UserActionRequired,
            disposition: RuntimeIssueDisposition::AskUserAndPause,
            code,
            side_effects_blocked: true,
            required_next_tool: None,
        }
    }

    pub const fn fatal(code: &'static str) -> Self {
        Self {
            severity: RuntimeIssueSeverity::Fatal,
            disposition: RuntimeIssueDisposition::AbortRun,
            code,
            side_effects_blocked: false,
            required_next_tool: None,
        }
    }

    pub const fn recoverable_required_next_tool(
        code: &'static str,
        required_next_tool: &'static str,
    ) -> Self {
        Self {
            severity: RuntimeIssueSeverity::Recoverable,
            disposition: RuntimeIssueDisposition::SteerModelAndContinue,
            code,
            side_effects_blocked: true,
            required_next_tool: Some(required_next_tool),
        }
    }
}

pub const fn checkpoint_follow_through_required_policy(
    required_next_tool: &'static str,
) -> RuntimeIssuePolicy {
    RuntimeIssuePolicy::recoverable_required_next_tool(
        "checkpoint_follow_through_required",
        required_next_tool,
    )
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
        "command_outcome_unknown" => ("command_outcome_unknown", false, "client_command"),
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
    if reason == "command_outcome_unknown" {
        return Some(format!(
            "{} could not confirm the command's final status after the connected client stopped responding. The command may still be running or may already have made changes. Reconnect and inspect the process and workspace before retrying it.",
            display_bear_name(bear_name)
        ));
    }
    if reason == "client_obligation_timeout" {
        return Some(format!(
            "{} stopped because the connected client did not respond in time. Reconnect the client and send another message to retry.",
            display_bear_name(bear_name)
        ));
    }
    if is_incomplete_stream(reason) {
        return Some(format!(
            "{} was interrupted before finishing. Your conversation and any completed tool results were preserved. Send another message to retry.",
            display_bear_name(bear_name)
        ));
    }
    if is_budget_or_loop_failure(reason, message) {
        return Some(format!(
            "{} stopped this turn after it ran too long. Recent tool results were preserved, but no final answer was delivered. Start a fresh turn to continue safely.",
            display_bear_name(bear_name)
        ));
    }
    if is_llm_provider_retry_exhausted(message) {
        return Some(format!(
            "{} could not reach the LLM provider after retrying this turn. Den already waited 2 seconds, then 4 seconds, then 54 seconds before giving up. You can send another message to retry, or close this session if you do not want to wait for another timeout.",
            display_bear_name(bear_name)
        ));
    }
    if is_llm_stream_idle_timeout(reason, message) {
        return Some(format!(
            "{} lost the LLM provider stream after it produced no data for 30 seconds. Recent tool results were preserved, but no final answer was delivered. You can send another message to retry, or close this session if you do not want to wait for another timeout.",
            display_bear_name(bear_name)
        ));
    }
    None
}

fn is_incomplete_stream(reason: &str) -> bool {
    matches!(
        reason,
        "stream_ended_without_runtime_terminal"
            | "continuation_stream_ended_without_runtime_terminal"
    )
}

fn is_llm_provider_retry_exhausted(message: &str) -> bool {
    message.contains("LLM provider hiccup persisted after")
        && message.contains("ending the turn after retry backoff")
}

fn is_llm_stream_idle_timeout(reason: &str, message: &str) -> bool {
    reason == "continuation_stream_error"
        && message.contains("LLM byte stream produced no data for 30s")
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
    } else if is_llm_provider_retry_exhausted(message) {
        diagnostic["class"] = json!("llm_provider_retry_exhausted");
        diagnostic["model_action"] = json!("report_retry_pause_and_wait_for_user_retry");
        diagnostic["retry_pauses_seconds"] = json!([2, 4, 54]);
    } else if is_llm_stream_idle_timeout(reason, message) {
        diagnostic["class"] = json!("llm_stream_idle_timeout");
        diagnostic["model_action"] = json!("wait_for_user_retry_after_reporting_stream_timeout");
        diagnostic["idle_timeout_seconds"] = json!(30);
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
    fn runtime_issue_policy_classifies_checkpoint_follow_through_as_recoverable() {
        let policy = checkpoint_follow_through_required_policy("checkpoint");

        assert_eq!(policy.severity, RuntimeIssueSeverity::Recoverable);
        assert_eq!(
            policy.disposition,
            RuntimeIssueDisposition::SteerModelAndContinue
        );
        assert_eq!(policy.code, "checkpoint_follow_through_required");
        assert!(policy.side_effects_blocked);
        assert_eq!(policy.required_next_tool, Some("checkpoint"));
    }

    #[test]
    fn runtime_issue_policy_keeps_user_pauses_and_fatals_distinct() {
        let pause = RuntimeIssuePolicy::user_action_required("confirmation_required");
        assert_eq!(pause.severity, RuntimeIssueSeverity::UserActionRequired);
        assert_eq!(pause.disposition, RuntimeIssueDisposition::AskUserAndPause);
        assert!(pause.side_effects_blocked);

        let fatal = RuntimeIssuePolicy::fatal("database_invariant_failed");
        assert_eq!(fatal.severity, RuntimeIssueSeverity::Fatal);
        assert_eq!(fatal.disposition, RuntimeIssueDisposition::AbortRun);
        assert!(!fatal.side_effects_blocked);
    }

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
    fn llm_provider_retry_exhaustion_has_friendly_user_message_and_diagnostics() {
        let message = "LLM provider hiccup persisted after 4 attempts for responses_stream model openai/gpt-5.5; retry pauses were 2s, 4s, and 54s; ending the turn after retry backoff. Last error: HTTP 503 Service Unavailable: overloaded";

        let projection =
            run_failure_projection("runtime_internal", message, "run-1", "Builder Bear", None);

        let user_message = projection.user_message.expect("user message");
        assert!(user_message.contains("waited 2 seconds, then 4 seconds, then 54 seconds"));
        assert!(user_message.contains("send another message to retry"));
        assert!(user_message.contains("close this session"));

        let context = projection.diagnostic_context.expect("diagnostic context");
        assert_eq!(
            context["diagnostic"]["class"],
            "llm_provider_retry_exhausted"
        );
        assert_eq!(
            context["diagnostic"]["model_action"],
            "report_retry_pause_and_wait_for_user_retry"
        );
        assert_eq!(
            context["diagnostic"]["retry_pauses_seconds"],
            json!([2, 4, 54])
        );
    }

    #[test]
    fn llm_stream_idle_timeout_has_friendly_user_message_and_diagnostics() {
        let message = "Server Error: LLM byte stream produced no data for 30s";

        let projection = run_failure_projection(
            "continuation_stream_error",
            message,
            "run-1",
            "Builder Bear",
            None,
        );

        let user_message = projection.user_message.expect("user message");
        assert!(user_message.contains("produced no data for 30 seconds"));
        assert!(user_message.contains("send another message to retry"));
        assert!(user_message.contains("close this session"));

        let context = projection.diagnostic_context.expect("diagnostic context");
        assert_eq!(context["diagnostic"]["class"], "llm_stream_idle_timeout");
        assert_eq!(
            context["diagnostic"]["model_action"],
            "wait_for_user_retry_after_reporting_stream_timeout"
        );
        assert_eq!(context["diagnostic"]["idle_timeout_seconds"], 30);
    }

    #[test]
    fn incomplete_stream_has_safe_retry_message() {
        let projection = run_failure_projection(
            "stream_ended_without_runtime_terminal",
            "The model stream ended after non-terminal runtime events.",
            "run-1",
            "Builder Bear",
            Some(json!({
                "runtime_event_count": 31,
                "pending_tool_call_ids": ["tool-5"],
            })),
        );

        let user_message = projection.user_message.expect("user message");
        assert!(user_message.contains("interrupted before finishing"));
        assert!(user_message.contains("Send another message to retry"));
        assert!(!user_message.contains("runtime_event_count"));
        assert!(projection.history_marker.is_some());
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
