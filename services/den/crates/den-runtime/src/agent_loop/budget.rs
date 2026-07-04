use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::ChatToolCall;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnBudgetPolicy {
    pub soft_steps: u32,
    pub hard_steps: u32,
    pub max_consecutive_tool_failures: u32,
    pub max_same_tool_signature_repeats: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnBudgetState {
    pub consecutive_tool_failures: u32,
    pub consecutive_non_progress_steps: u32,
    pub last_tool_signature: Option<String>,
    pub same_tool_signature_repeats: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolContinuationObservation {
    pub tool_name: String,
    pub signature: String,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnBudgetEvaluation {
    pub next_state: TurnBudgetState,
    pub progress: bool,
    pub stop_reason: Option<TurnBudgetStopReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnBudgetStopReason {
    HardStepLimit {
        step: u32,
        hard_steps: u32,
    },
    SoftStepNoProgress {
        step: u32,
        soft_steps: u32,
        hard_steps: u32,
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
}

impl TurnBudgetStopReason {
    pub fn persistence_reason(&self) -> &'static str {
        match self {
            Self::HardStepLimit { .. } => "hard_step_limit",
            Self::SoftStepNoProgress { .. } => "soft_step_no_progress",
            Self::ConsecutiveToolFailures { .. } => "consecutive_tool_failures",
            Self::RuleOfKo { .. } => "rule_of_ko",
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::HardStepLimit { step, hard_steps } => format!(
                "I stopped because this turn reached the hard tool/permission continuation budget before producing a final answer (step={step}/hard_steps={hard_steps}). The recent tool results were recorded, but this run needs a fresh turn to continue safely."
            ),
            Self::SoftStepNoProgress {
                step,
                soft_steps,
                hard_steps,
            } => format!(
                "I stopped because this turn exhausted its exploratory tool budget without making enough forward progress (step={step}/soft_steps={soft_steps}, hard_steps={hard_steps}). The recent tool results were recorded, but the model appears to be churning instead of narrowing the task. Please retry with a narrower request or point me at the exact file/path to inspect."
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
        }
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
    prior_state: &TurnBudgetState,
    observations: &[ToolContinuationObservation],
) -> TurnBudgetEvaluation {
    let unique_signatures = unique_signatures(observations);
    let primary_tool_name = observations
        .first()
        .map(|observation| observation.tool_name.clone());
    let primary_signature = if unique_signatures.len() == 1 {
        unique_signatures.first().cloned()
    } else {
        None
    };
    let progress = match primary_signature.as_deref() {
        Some(signature) => prior_state.last_tool_signature.as_deref() != Some(signature),
        None => !observations.is_empty(),
    };

    let mut next_state = prior_state.clone();
    next_state.consecutive_non_progress_steps = if progress {
        0
    } else {
        prior_state.consecutive_non_progress_steps.saturating_add(1)
    };

    let batch_failed =
        !observations.is_empty() && observations.iter().all(|observation| observation.failed);
    next_state.consecutive_tool_failures = if batch_failed {
        prior_state.consecutive_tool_failures.saturating_add(1)
    } else {
        0
    };

    match primary_signature {
        Some(signature) => {
            next_state.same_tool_signature_repeats =
                if prior_state.last_tool_signature.as_deref() == Some(signature.as_str()) {
                    prior_state.same_tool_signature_repeats.saturating_add(1)
                } else {
                    1
                };
            next_state.last_tool_signature = Some(signature);
        }
        None => {
            next_state.same_tool_signature_repeats = 0;
            if !unique_signatures.is_empty() {
                next_state.last_tool_signature = Some(unique_signatures.join(" || "));
            }
        }
    }

    let stop_reason = if step >= policy.hard_steps {
        Some(TurnBudgetStopReason::HardStepLimit {
            step,
            hard_steps: policy.hard_steps,
        })
    } else if next_state.same_tool_signature_repeats > policy.max_same_tool_signature_repeats {
        Some(TurnBudgetStopReason::RuleOfKo {
            repeats: next_state.same_tool_signature_repeats,
            limit: policy.max_same_tool_signature_repeats,
            tool_name: primary_tool_name,
        })
    } else if next_state.consecutive_tool_failures >= policy.max_consecutive_tool_failures {
        Some(TurnBudgetStopReason::ConsecutiveToolFailures {
            count: next_state.consecutive_tool_failures,
            limit: policy.max_consecutive_tool_failures,
            tool_name: primary_tool_name,
        })
    } else if step >= policy.soft_steps && next_state.consecutive_non_progress_steps > 0 {
        Some(TurnBudgetStopReason::SoftStepNoProgress {
            step,
            soft_steps: policy.soft_steps,
            hard_steps: policy.hard_steps,
        })
    } else {
        None
    };

    TurnBudgetEvaluation {
        next_state,
        progress,
        stop_reason,
    }
}

fn unique_signatures(observations: &[ToolContinuationObservation]) -> Vec<String> {
    let mut unique = Vec::new();
    for observation in observations {
        if !unique
            .iter()
            .any(|existing| existing == &observation.signature)
        {
            unique.push(observation.signature.clone());
        }
    }
    unique
}

fn canonicalize_jsonish(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(value) => canonical_json(&value),
        Err(_) => trimmed.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    append_canonical_json(value, &mut out);
    out
}

fn append_canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(boolean) => out.push_str(if *boolean { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(string) => {
            out.push_str(&serde_json::to_string(string).expect("json string serialization"));
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                append_canonical_json(item, out);
            }
            out.push(']');
        }
        Value::Object(object) => {
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

    fn observation(tool_name: &str, arguments: &str, failed: bool) -> ToolContinuationObservation {
        ToolContinuationObservation {
            tool_name: tool_name.to_string(),
            signature: tool_signature(tool_name, arguments),
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
        let policy = TurnBudgetPolicy {
            soft_steps: 6,
            hard_steps: 12,
            max_consecutive_tool_failures: 3,
            max_same_tool_signature_repeats: 2,
        };
        let prior = TurnBudgetState {
            last_tool_signature: Some(tool_signature("memory_read", r#"{\"path\":\"a\"}"#)),
            same_tool_signature_repeats: 2,
            ..Default::default()
        };

        let evaluation = evaluate_turn_budget(
            policy,
            3,
            &prior,
            &[observation("memory_read", r#"{\"path\":\"a\"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::RuleOfKo { repeats: 3, .. })
        ));
    }

    #[test]
    fn hard_budget_allows_last_permitted_step_and_stops_afterward() {
        let policy = TurnBudgetPolicy {
            soft_steps: 6,
            hard_steps: 12,
            max_consecutive_tool_failures: 3,
            max_same_tool_signature_repeats: 5,
        };
        let prior = TurnBudgetState {
            last_tool_signature: Some(tool_signature("memory_read", r#"{\"path\":\"a\"}"#)),
            ..Default::default()
        };

        let before_limit = evaluate_turn_budget(
            policy,
            11,
            &prior,
            &[observation("memory_read", r#"{\"path\":\"b\"}"#, false)],
        );
        assert!(before_limit.stop_reason.is_none());

        let at_limit = evaluate_turn_budget(
            policy,
            12,
            &prior,
            &[observation("memory_read", r#"{\"path\":\"c\"}"#, false)],
        );
        assert!(matches!(
            at_limit.stop_reason,
            Some(TurnBudgetStopReason::HardStepLimit {
                step: 12,
                hard_steps: 12,
            })
        ));
    }

    #[test]
    fn soft_budget_allows_extension_when_signature_changes() {
        let policy = TurnBudgetPolicy {
            soft_steps: 6,
            hard_steps: 12,
            max_consecutive_tool_failures: 3,
            max_same_tool_signature_repeats: 2,
        };
        let prior = TurnBudgetState {
            last_tool_signature: Some(tool_signature("memory_read", r#"{\"path\":\"a\"}"#)),
            ..Default::default()
        };

        let evaluation = evaluate_turn_budget(
            policy,
            5,
            &prior,
            &[observation("memory_read", r#"{\"path\":\"b\"}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert!(evaluation.progress);
    }

    #[test]
    fn soft_budget_stops_non_progress_after_threshold() {
        let policy = TurnBudgetPolicy {
            soft_steps: 6,
            hard_steps: 12,
            max_consecutive_tool_failures: 3,
            max_same_tool_signature_repeats: 5,
        };
        let prior = TurnBudgetState {
            last_tool_signature: Some(tool_signature("memory_read", r#"{\"path\":\"a\"}"#)),
            ..Default::default()
        };

        let evaluation = evaluate_turn_budget(
            policy,
            6,
            &prior,
            &[observation("memory_read", r#"{\"path\":\"a\"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::SoftStepNoProgress { .. })
        ));
    }

    #[test]
    fn consecutive_failures_trigger_failure_budget() {
        let policy = TurnBudgetPolicy {
            soft_steps: 6,
            hard_steps: 12,
            max_consecutive_tool_failures: 3,
            max_same_tool_signature_repeats: 5,
        };
        let prior = TurnBudgetState {
            consecutive_tool_failures: 2,
            ..Default::default()
        };

        let evaluation = evaluate_turn_budget(
            policy,
            2,
            &prior,
            &[observation("memory_read", r#"{\"path\":\"a\"}"#, true)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::ConsecutiveToolFailures { count: 3, .. })
        ));
    }
}
