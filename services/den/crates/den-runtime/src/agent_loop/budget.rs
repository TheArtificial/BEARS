use std::time::Instant;

use crate::llm::ChatToolCall;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCallBudgetLimits {
    pub total: u32,
    pub read: u32,
    pub search: u32,
    pub fetch: u32,
    pub execute: u32,
    pub write: u32,
    pub destructive: u32,
    pub other: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCallBudgetUsage {
    pub total: u32,
    pub read: u32,
    pub search: u32,
    pub fetch: u32,
    pub execute: u32,
    pub write: u32,
    pub destructive: u32,
    pub other: u32,
}

impl ToolCallBudgetUsage {
    fn increment(&mut self, class: ToolBudgetClass) {
        self.total = self.total.saturating_add(1);
        match class {
            ToolBudgetClass::Read => self.read = self.read.saturating_add(1),
            ToolBudgetClass::Search => self.search = self.search.saturating_add(1),
            ToolBudgetClass::Fetch => self.fetch = self.fetch.saturating_add(1),
            ToolBudgetClass::Execute => self.execute = self.execute.saturating_add(1),
            ToolBudgetClass::Write => self.write = self.write.saturating_add(1),
            ToolBudgetClass::Destructive => self.destructive = self.destructive.saturating_add(1),
            ToolBudgetClass::Other => self.other = self.other.saturating_add(1),
        }
    }

    fn count_for(self, class: ToolBudgetClass) -> u32 {
        match class {
            ToolBudgetClass::Read => self.read,
            ToolBudgetClass::Search => self.search,
            ToolBudgetClass::Fetch => self.fetch,
            ToolBudgetClass::Execute => self.execute,
            ToolBudgetClass::Write => self.write,
            ToolBudgetClass::Destructive => self.destructive,
            ToolBudgetClass::Other => self.other,
        }
    }

    fn limit_for(limits: ToolCallBudgetLimits, class: ToolBudgetClass) -> u32 {
        match class {
            ToolBudgetClass::Read => limits.read,
            ToolBudgetClass::Search => limits.search,
            ToolBudgetClass::Fetch => limits.fetch,
            ToolBudgetClass::Execute => limits.execute,
            ToolBudgetClass::Write => limits.write,
            ToolBudgetClass::Destructive => limits.destructive,
            ToolBudgetClass::Other => limits.other,
        }
    }
}

impl Default for ToolCallBudgetUsage {
    fn default() -> Self {
        Self {
            total: 0,
            read: 0,
            search: 0,
            fetch: 0,
            execute: 0,
            write: 0,
            destructive: 0,
            other: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolBudgetClass {
    Read,
    Search,
    Fetch,
    Execute,
    Write,
    Destructive,
    Other,
}

impl ToolBudgetClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Search => "search",
            Self::Fetch => "fetch",
            Self::Execute => "execute",
            Self::Write => "write",
            Self::Destructive => "destructive",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnBudgetPolicy {
    pub max_wall_clock_ms: u64,
    pub emergency_hard_steps: u32,
    pub tool_call_limits: ToolCallBudgetLimits,
    pub max_consecutive_tool_failures: u32,
    pub max_same_tool_signature_repeats: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnBudgetState {
    pub started_at: Instant,
    pub tool_usage: ToolCallBudgetUsage,
    pub consecutive_tool_failures: u32,
    pub last_batch_signature: Option<String>,
    pub same_batch_signature_repeats: u32,
}

impl Default for TurnBudgetState {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            tool_usage: ToolCallBudgetUsage::default(),
            consecutive_tool_failures: 0,
            last_batch_signature: None,
            same_batch_signature_repeats: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolContinuationObservation {
    pub tool_name: String,
    pub signature: String,
    pub class: ToolBudgetClass,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnBudgetEvaluation {
    pub next_state: TurnBudgetState,
    pub warning: Option<TurnBudgetWarning>,
    pub stop_reason: Option<TurnBudgetStopReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnBudgetWarning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnBudgetStopReason {
    WallClockLimit {
        elapsed_ms: u64,
        limit_ms: u64,
    },
    TotalToolCallLimit {
        count: u32,
        limit: u32,
    },
    ToolClassCallLimit {
        class: ToolBudgetClass,
        count: u32,
        limit: u32,
    },
    ConsecutiveToolFailures {
        count: u32,
        limit: u32,
        tool_name: Option<String>,
    },
    RuleOfKo {
        repeats: u32,
        limit: u32,
        tool_name: Option<String>,
    },
    EmergencyHardStepLimit {
        step: u32,
        emergency_hard_steps: u32,
    },
}

impl TurnBudgetWarning {
    pub fn model_message(&self) -> &str {
        &self.message
    }
}

impl TurnBudgetStopReason {
    pub fn persistence_reason(&self) -> &'static str {
        match self {
            Self::WallClockLimit { .. } => "wall_clock_limit",
            Self::TotalToolCallLimit { .. } => "total_tool_call_limit",
            Self::ToolClassCallLimit { .. } => "tool_class_call_limit",
            Self::ConsecutiveToolFailures { .. } => "consecutive_tool_failures",
            Self::RuleOfKo { .. } => "rule_of_ko",
            Self::EmergencyHardStepLimit { .. } => "emergency_hard_step_limit",
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::WallClockLimit { elapsed_ms, limit_ms } => format!(
                "I stopped because this turn exhausted its wall-clock budget (elapsed={}ms/limit={}ms). The recent tool results were recorded, but this run needs a fresh turn to continue safely.",
                elapsed_ms, limit_ms
            ),
            Self::TotalToolCallLimit { count, limit } => format!(
                "I stopped because this turn exhausted its emergency total tool-call fuse (tool_calls={count}/limit={limit}). The recent tool results were recorded, but this run needs a fresh turn to continue safely."
            ),
            Self::ToolClassCallLimit { class, count, limit } => format!(
                "I stopped because this turn exhausted its {} tool budget ({}_calls={}/limit={}). The recent tool results were recorded, but this run needs a fresh turn to continue safely.",
                class.label(),
                class.label(),
                count,
                limit
            ),
            Self::ConsecutiveToolFailures {
                count,
                limit,
                tool_name,
            } => {
                let tool = tool_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .map(|name| format!(" for `{name}`"))
                    .unwrap_or_default();
                format!(
                    "I stopped because this turn hit {count} consecutive tool failures{tool} (limit={limit}). The recent tool results were recorded, but the model appears to be stuck retrying failures instead of recovering."
                )
            }
            Self::RuleOfKo {
                repeats,
                limit,
                tool_name,
            } => {
                let tool = tool_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .map(|name| format!(" for `{name}`"))
                    .unwrap_or_default();
                format!(
                    "I stopped because this turn repeated the same tool-call pattern{tool} without changing the search state (rule of ko, repeats={repeats}/limit={limit}). The recent tool results were recorded, but the model appears to be looping."
                )
            }
            Self::EmergencyHardStepLimit {
                step,
                emergency_hard_steps,
            } => format!(
                "I stopped because this turn hit the emergency continuation fuse (step={step}/emergency_hard_steps={emergency_hard_steps}). The recent tool results were recorded, but this run needs a fresh turn to continue safely."
            ),
        }
    }
}

pub fn classify_tool_budget_class(tool_name: &str) -> ToolBudgetClass {
    match tool_name.trim() {
        "fs_find_paths" | "fs_search_files" | "memory_search" | "web_search" => {
            ToolBudgetClass::Search
        }
        "web_fetch" | "local_web_fetch" | "chrome_open" => ToolBudgetClass::Fetch,
        "terminal_run_command" | "process_run" => ToolBudgetClass::Execute,
        "fs_edit_file"
        | "fs_replace_text"
        | "fs_create_text_file"
        | "fs_create_directory"
        | "fs_move_path"
        | "fs_copy_path"
        | "fs_apply_patch"
        | "git_add"
        | "git_restore"
        | "git_commit"
        | "git_stash"
        | "memory_write_entry"
        | "memory_request_review"
        | "update_task_list"
        | "request_task_list_handoff"
        | "set_conversation_title" => ToolBudgetClass::Write,
        "fs_delete_path" => ToolBudgetClass::Destructive,
        "fs_read_text_file"
        | "fs_list_directory"
        | "fs_stat"
        | "git_status"
        | "git_diff"
        | "git_log"
        | "git_show"
        | "memory_read"
        | "memory_browse"
        | "session_info"
        | "list_task_lists"
        | "get_task_list_status"
        | "bear_environment"
        | "chrome_snapshot"
        | "chrome_console_messages"
        | "chrome_network_requests"
        | "chrome_screenshot" => ToolBudgetClass::Read,
        _ => ToolBudgetClass::Other,
    }
}

pub fn tool_signature_from_call(call: &ChatToolCall) -> String {
    tool_signature(&call.function.name, &call.function.arguments)
}

pub fn tool_signature(tool_name: &str, arguments: &str) -> String {
    let args = canonicalize_jsonish(arguments);
    format!("{}|{}", tool_name.trim(), args)
}

pub fn evaluate_turn_budget(
    policy: TurnBudgetPolicy,
    step: u32,
    elapsed_ms: u64,
    prior_state: &TurnBudgetState,
    observations: &[ToolContinuationObservation],
) -> TurnBudgetEvaluation {
    let mut next_state = prior_state.clone();
    for observation in observations {
        next_state.tool_usage.increment(observation.class);
    }

    let batch_signature = batch_signature(observations);
    let primary_tool_name = observations
        .first()
        .map(|observation| observation.tool_name.clone());

    if let Some(signature) = batch_signature {
        next_state.same_batch_signature_repeats =
            if prior_state.last_batch_signature.as_deref() == Some(signature.as_str()) {
                prior_state.same_batch_signature_repeats.saturating_add(1)
            } else {
                1
            };
        next_state.last_batch_signature = Some(signature);
    }

    let batch_failed =
        !observations.is_empty() && observations.iter().all(|observation| observation.failed);
    if batch_failed {
        next_state.consecutive_tool_failures =
            prior_state.consecutive_tool_failures.saturating_add(1);
    } else if !observations.is_empty() {
        next_state.consecutive_tool_failures = 0;
    }

    let stop_reason = if elapsed_ms >= policy.max_wall_clock_ms {
        Some(TurnBudgetStopReason::WallClockLimit {
            elapsed_ms,
            limit_ms: policy.max_wall_clock_ms,
        })
    } else if next_state.same_batch_signature_repeats > policy.max_same_tool_signature_repeats {
        Some(TurnBudgetStopReason::RuleOfKo {
            repeats: next_state.same_batch_signature_repeats,
            limit: policy.max_same_tool_signature_repeats,
            tool_name: primary_tool_name,
        })
    } else if next_state.consecutive_tool_failures >= policy.max_consecutive_tool_failures {
        Some(TurnBudgetStopReason::ConsecutiveToolFailures {
            count: next_state.consecutive_tool_failures,
            limit: policy.max_consecutive_tool_failures,
            tool_name: primary_tool_name,
        })
    } else if next_state.tool_usage.total > policy.tool_call_limits.total {
        Some(TurnBudgetStopReason::TotalToolCallLimit {
            count: next_state.tool_usage.total,
            limit: policy.tool_call_limits.total,
        })
    } else if let Some((class, count, limit)) =
        first_exhausted_class(next_state.tool_usage, policy.tool_call_limits)
    {
        Some(TurnBudgetStopReason::ToolClassCallLimit {
            class,
            count,
            limit,
        })
    } else if step >= policy.emergency_hard_steps {
        Some(TurnBudgetStopReason::EmergencyHardStepLimit {
            step,
            emergency_hard_steps: policy.emergency_hard_steps,
        })
    } else {
        None
    };

    let warning = if stop_reason.is_none() {
        budget_warning(policy, step, elapsed_ms, &next_state)
    } else {
        None
    };

    TurnBudgetEvaluation {
        next_state,
        warning,
        stop_reason,
    }
}

fn budget_warning(
    policy: TurnBudgetPolicy,
    step: u32,
    elapsed_ms: u64,
    state: &TurnBudgetState,
) -> Option<TurnBudgetWarning> {
    if step + 1 >= policy.emergency_hard_steps {
        return Some(TurnBudgetWarning {
            code: "emergency_hard_step_warning",
            message: format!(
                "Budget advisory: this turn is at the end of its emergency continuation fuse (next step would reach {}/{}). If you already have enough information, stop calling tools and provide the best final answer now.",
                step + 1,
                policy.emergency_hard_steps
            ),
        });
    }

    let remaining_wall_clock_ms = policy.max_wall_clock_ms.saturating_sub(elapsed_ms);
    if remaining_wall_clock_ms <= 15_000
        || elapsed_ms.saturating_mul(100) >= policy.max_wall_clock_ms.saturating_mul(85)
    {
        return Some(TurnBudgetWarning {
            code: "wall_clock_warning",
            message: format!(
                "Budget advisory: this turn is close to its wall-clock limit (remaining={}ms). Prefer a final answer over more tool calls unless one more call is strictly necessary.",
                remaining_wall_clock_ms
            ),
        });
    }

    if state.tool_usage.total >= policy.tool_call_limits.total {
        return Some(TurnBudgetWarning {
            code: "total_tool_budget_warning",
            message: format!(
                "Budget advisory: this turn has fully used its emergency total tool-call fuse ({}/{} tool calls used). Any further tool call will stop the turn. Provide the best final answer now unless you explicitly need a fresh turn.",
                state.tool_usage.total,
                policy.tool_call_limits.total
            ),
        });
    }

    if state.tool_usage.total + 1 >= policy.tool_call_limits.total {
        return Some(TurnBudgetWarning {
            code: "total_tool_budget_warning",
            message: format!(
                "Budget advisory: this turn is close to its emergency total tool-call fuse ({}/{} tool calls used). Prefer a final answer over more tool calls unless one more call is strictly necessary.",
                state.tool_usage.total,
                policy.tool_call_limits.total
            ),
        });
    }

    for class in [
        ToolBudgetClass::Destructive,
        ToolBudgetClass::Write,
        ToolBudgetClass::Execute,
        ToolBudgetClass::Fetch,
        ToolBudgetClass::Search,
        ToolBudgetClass::Read,
        ToolBudgetClass::Other,
    ] {
        let count = state.tool_usage.count_for(class);
        let limit = ToolCallBudgetUsage::limit_for(policy.tool_call_limits, class);
        if count == 0 {
            continue;
        }
        if count >= limit {
            return Some(TurnBudgetWarning {
                code: "tool_class_budget_warning",
                message: format!(
                    "Budget advisory: this turn has fully used its {} tool budget ({}/{} used). Any further {} call will stop the turn. Provide the best final answer now unless you explicitly need a fresh turn.",
                    class.label(),
                    count,
                    limit,
                    class.label()
                ),
            });
        }
        if count + 1 >= limit {
            return Some(TurnBudgetWarning {
                code: "tool_class_budget_warning",
                message: format!(
                    "Budget advisory: this turn is close to its {} tool budget ({}/{} used). Prefer a final answer over more tool calls unless one more {} call is strictly necessary.",
                    class.label(),
                    count,
                    limit,
                    class.label()
                ),
            });
        }
    }

    if state.consecutive_tool_failures + 1 >= policy.max_consecutive_tool_failures {
        return Some(TurnBudgetWarning {
            code: "failure_budget_warning",
            message: format!(
                "Budget advisory: this turn is close to its repeated-failure limit ({}/{} consecutive failed tool batches). Do not retry the same failing action unless you have new evidence.",
                state.consecutive_tool_failures,
                policy.max_consecutive_tool_failures
            ),
        });
    }

    if state.same_batch_signature_repeats + 1 >= policy.max_same_tool_signature_repeats {
        return Some(TurnBudgetWarning {
            code: "rule_of_ko_warning",
            message: format!(
                "Budget advisory: this turn is close to its loop-ko limit ({}/{} repeated tool batches). Do not repeat the same tool call pattern again; either answer now or choose a materially different next step.",
                state.same_batch_signature_repeats,
                policy.max_same_tool_signature_repeats
            ),
        });
    }

    None
}

fn first_exhausted_class(
    usage: ToolCallBudgetUsage,
    limits: ToolCallBudgetLimits,
) -> Option<(ToolBudgetClass, u32, u32)> {
    for class in [
        ToolBudgetClass::Destructive,
        ToolBudgetClass::Write,
        ToolBudgetClass::Execute,
        ToolBudgetClass::Fetch,
        ToolBudgetClass::Search,
        ToolBudgetClass::Read,
        ToolBudgetClass::Other,
    ] {
        let count = usage.count_for(class);
        let limit = ToolCallBudgetUsage::limit_for(limits, class);
        if count > limit {
            return Some((class, count, limit));
        }
    }
    None
}

fn batch_signature(observations: &[ToolContinuationObservation]) -> Option<String> {
    if observations.is_empty() {
        return None;
    }
    Some(
        observations
            .iter()
            .map(|observation| observation.signature.as_str())
            .collect::<Vec<_>>()
            .join(" || "),
    )
}

fn canonicalize_jsonish(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => canonical_json(&value),
        Err(_) => trimmed.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

fn canonical_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    append_canonical_json(value, &mut out);
    out
}

fn append_canonical_json(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(boolean) => out.push_str(if *boolean { "true" } else { "false" }),
        serde_json::Value::Number(number) => out.push_str(&number.to_string()),
        serde_json::Value::String(string) => {
            out.push_str(&serde_json::to_string(string).expect("json string serialization"));
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                append_canonical_json(item, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(object) => {
            out.push('{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("json object key serialization"));
                out.push(':');
                append_canonical_json(&object[key], out);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> TurnBudgetPolicy {
        TurnBudgetPolicy {
            max_wall_clock_ms: 60_000,
            emergency_hard_steps: 32,
            tool_call_limits: ToolCallBudgetLimits {
                total: 20,
                read: 12,
                search: 8,
                fetch: 4,
                execute: 4,
                write: 4,
                destructive: 1,
                other: 6,
            },
            max_consecutive_tool_failures: 3,
            max_same_tool_signature_repeats: 2,
        }
    }

    fn state() -> TurnBudgetState {
        TurnBudgetState {
            started_at: Instant::now(),
            ..Default::default()
        }
    }

    fn observation(tool_name: &str, arguments: &str, failed: bool) -> ToolContinuationObservation {
        ToolContinuationObservation {
            tool_name: tool_name.to_string(),
            signature: tool_signature(tool_name, arguments),
            class: classify_tool_budget_class(tool_name),
            failed,
        }
    }

    #[test]
    fn tool_signature_canonicalizes_json_argument_order() {
        assert_eq!(
            tool_signature("memory_read", r#"{"b":2,"a":1}"#),
            tool_signature("memory_read", r#"{"a":1,"b":2}"#)
        );
    }

    #[test]
    fn repeated_same_signature_triggers_rule_of_ko() {
        let mut prior = state();
        prior.last_batch_signature = Some(tool_signature("memory_read", r#"{"path":"a"}"#));
        prior.same_batch_signature_repeats = 2;

        let evaluation = evaluate_turn_budget(
            policy(),
            3,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::RuleOfKo { repeats: 3, .. })
        ));
    }

    #[test]
    fn wall_clock_budget_stops_even_when_tool_counts_are_low() {
        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            60_000,
            &state(),
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::WallClockLimit { .. })
        ));
    }

    #[test]
    fn read_budget_exhaustion_stops_before_step_fuse() {
        let mut prior = state();
        prior.tool_usage.read = 12;
        prior.tool_usage.total = 12;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::ToolClassCallLimit {
                class: ToolBudgetClass::Read,
                count: 13,
                limit: 12,
            })
        ));
    }

    #[test]
    fn total_tool_budget_stops_after_limit() {
        let mut prior = state();
        prior.tool_usage.total = 20;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::TotalToolCallLimit {
                count: 21,
                limit: 20
            })
        ));
    }

    #[test]
    fn total_tool_budget_warns_before_limit() {
        let mut prior = state();
        prior.tool_usage.total = 19;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert_eq!(
            evaluation.warning.as_ref().map(|warning| warning.code),
            Some("total_tool_budget_warning")
        );
        assert!(evaluation
            .warning
            .as_ref()
            .is_some_and(|warning| warning.message.contains("Any further tool call will stop the turn")));
    }

    #[test]
    fn read_budget_warns_when_last_safe_read_is_already_used() {
        let mut prior = state();
        prior.tool_usage.read = 11;
        prior.tool_usage.total = 11;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert_eq!(
            evaluation.warning.as_ref().map(|warning| warning.code),
            Some("tool_class_budget_warning")
        );
        assert!(evaluation.warning.as_ref().is_some_and(|warning| warning
            .message
            .contains("Any further read call will stop the turn")));
    }

    #[test]
    fn class_budget_warning_ignores_unused_stricter_classes() {
        let mut prior = state();
        prior.tool_usage.read = 10;
        prior.tool_usage.total = 10;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert!(evaluation.warning.as_ref().is_some_and(|warning| {
            warning.code == "tool_class_budget_warning"
                && warning.message.contains("read tool budget")
        }));
    }

    #[test]
    fn consecutive_failures_trigger_failure_budget() {
        let mut prior = state();
        prior.consecutive_tool_failures = 2;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, true)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::ConsecutiveToolFailures { count: 3, .. })
        ));
    }

    #[test]
    fn emergency_hard_step_limit_is_only_a_last_resort_fuse() {
        let evaluation = evaluate_turn_budget(
            policy(),
            32,
            1_000,
            &state(),
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::EmergencyHardStepLimit {
                step: 32,
                emergency_hard_steps: 32,
            })
        ));
    }

    #[test]
    fn emergency_hard_step_warns_one_step_early() {
        let evaluation = evaluate_turn_budget(
            policy(),
            31,
            1_000,
            &state(),
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert_eq!(
            evaluation.warning.as_ref().map(|warning| warning.code),
            Some("emergency_hard_step_warning")
        );
    }
}
