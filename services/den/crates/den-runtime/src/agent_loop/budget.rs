use std::time::Instant;

use den_protocol::ContextBudgetReport;
use den_service::bears::prompt_fragments::{
    render_turn_fragment, repository_prompt_fragment_registry,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::llm::ChatToolCall;

use super::checkpoints::GroundingProbeSignalKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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

impl ToolCallBudgetLimits {
    fn class_limits(self) -> [u32; TOOL_BUDGET_CLASS_COUNT] {
        [
            self.read,
            self.search,
            self.fetch,
            self.execute,
            self.write,
            self.destructive,
            self.other,
        ]
    }

    fn limit_for(self, class: ToolBudgetClass) -> u32 {
        self.class_limits()[class.index()]
    }
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

    fn class_counts(self) -> [u32; TOOL_BUDGET_CLASS_COUNT] {
        [
            self.read,
            self.search,
            self.fetch,
            self.execute,
            self.write,
            self.destructive,
            self.other,
        ]
    }

    fn count_for(self, class: ToolBudgetClass) -> u32 {
        self.class_counts()[class.index()]
    }
}

const TOOL_BUDGET_CLASS_COUNT: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBudgetClass {
    Read,
    Search,
    Fetch,
    Execute,
    Write,
    Destructive,
    Other,
}

const ALL_BUDGET_CLASSES_BY_SEVERITY: &[ToolBudgetClass] = &[
    ToolBudgetClass::Destructive,
    ToolBudgetClass::Write,
    ToolBudgetClass::Execute,
    ToolBudgetClass::Fetch,
    ToolBudgetClass::Search,
    ToolBudgetClass::Read,
    ToolBudgetClass::Other,
];

impl ToolBudgetClass {
    const fn index(self) -> usize {
        match self {
            Self::Read => 0,
            Self::Search => 1,
            Self::Fetch => 2,
            Self::Execute => 3,
            Self::Write => 4,
            Self::Destructive => 5,
            Self::Other => 6,
        }
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnBudgetPolicy {
    pub max_wall_clock_ms: u64,
    pub emergency_hard_steps: u32,
    pub tool_call_limits: ToolCallBudgetLimits,
    pub max_consecutive_tool_failures: u32,
    pub max_same_tool_signature_repeats: u32,
    pub post_mutation_verification_window: Option<PostMutationVerificationWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostMutationVerificationWindow {
    pub replenish_read: u32,
    pub replenish_search: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnBudgetState {
    pub started_at: Instant,
    pub tool_usage: ToolCallBudgetUsage,
    pub latest_context_budget: Option<ContextBudgetReport>,
    pub consecutive_tool_failures: u32,
    pub last_batch_signature: Option<String>,
    pub same_batch_signature_repeats: u32,
    /// A one-shot escape hatch after a just-completed tool result pushes the turn over a budget.
    /// The model gets one finalization continuation to consume the result and answer without
    /// tools; another over-budget tool continuation hard-stops the turn.
    pub budget_finalization_grace_used: bool,
}

impl Default for TurnBudgetState {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            tool_usage: ToolCallBudgetUsage::default(),
            latest_context_budget: None,
            consecutive_tool_failures: 0,
            last_batch_signature: None,
            same_batch_signature_repeats: 0,
            budget_finalization_grace_used: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolContinuationObservation {
    pub tool_name: String,
    pub signature: String,
    pub class: ToolBudgetClass,
    pub failed: bool,
    pub grounding_probe_signal: Option<GroundingProbeSignalKind>,
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
    ContextBudgetLimit {
        model: String,
        estimated_input_tokens: u32,
        reserved_output_tokens: u32,
        estimated_total_tokens: u32,
        context_window: Option<u32>,
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
            Self::ContextBudgetLimit { .. } => "context_budget_limit",
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::WallClockLimit { .. } => "I ran out of execution time after recording the work completed so far. Send “continue” and I’ll pick up from the recorded state.".to_string(),
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
            Self::ContextBudgetLimit {
                model,
                estimated_input_tokens,
                reserved_output_tokens,
                estimated_total_tokens,
                context_window,
            } => format!(
                "I stopped before the next model call because the compiled request exceeds the context budget for {model} (estimated_input_tokens={estimated_input_tokens}, reserved_output_tokens={reserved_output_tokens}, estimated_total_tokens={estimated_total_tokens}, context_window={}). Compact or reduce context before continuing.",
                context_window.unwrap_or_default()
            ),
        }
    }
}

fn observation_is_successful_mutation(observation: &ToolContinuationObservation) -> bool {
    !observation.failed
        && observation.grounding_probe_signal != Some(GroundingProbeSignalKind::Fail)
        && matches!(
            observation.class,
            ToolBudgetClass::Write | ToolBudgetClass::Destructive
        )
}

fn apply_post_mutation_verification_window(
    usage: &mut ToolCallBudgetUsage,
    window: PostMutationVerificationWindow,
) {
    usage.read = usage.read.saturating_sub(window.replenish_read);
    usage.search = usage.search.saturating_sub(window.replenish_search);
}

const TOOL_BUDGET_CLASS_BY_NAME: &[(&str, ToolBudgetClass)] = &[
    ("fs_find_paths", ToolBudgetClass::Search),
    ("fs_search_files", ToolBudgetClass::Search),
    ("memory_search", ToolBudgetClass::Search),
    ("web_search", ToolBudgetClass::Search),
    ("web_fetch", ToolBudgetClass::Fetch),
    ("local_web_fetch", ToolBudgetClass::Fetch),
    ("chrome_open", ToolBudgetClass::Fetch),
    ("terminal_run_command", ToolBudgetClass::Execute),
    ("process_run", ToolBudgetClass::Execute),
    ("fs_edit_file", ToolBudgetClass::Write),
    ("fs_replace_text", ToolBudgetClass::Write),
    ("fs_create_text_file", ToolBudgetClass::Write),
    ("fs_create_directory", ToolBudgetClass::Write),
    ("fs_move_path", ToolBudgetClass::Write),
    ("fs_copy_path", ToolBudgetClass::Write),
    ("fs_apply_patch", ToolBudgetClass::Write),
    ("git_add", ToolBudgetClass::Write),
    ("git_restore", ToolBudgetClass::Write),
    ("git_commit", ToolBudgetClass::Write),
    ("git_stash", ToolBudgetClass::Write),
    ("memory_write_entry", ToolBudgetClass::Write),
    ("memory_request_review", ToolBudgetClass::Write),
    ("update_task_list", ToolBudgetClass::Write),
    ("request_task_list_handoff", ToolBudgetClass::Write),
    ("set_conversation_title", ToolBudgetClass::Write),
    ("fs_delete_path", ToolBudgetClass::Destructive),
    ("fs_read_text_file", ToolBudgetClass::Read),
    ("fs_list_directory", ToolBudgetClass::Read),
    ("fs_stat", ToolBudgetClass::Read),
    ("git_status", ToolBudgetClass::Read),
    ("git_diff", ToolBudgetClass::Read),
    ("git_log", ToolBudgetClass::Read),
    ("git_show", ToolBudgetClass::Read),
    ("memory_read", ToolBudgetClass::Read),
    ("memory_browse", ToolBudgetClass::Read),
    ("session_info", ToolBudgetClass::Read),
    ("list_task_lists", ToolBudgetClass::Read),
    ("get_task_list_status", ToolBudgetClass::Read),
    ("bear_environment", ToolBudgetClass::Read),
    ("chrome_snapshot", ToolBudgetClass::Read),
    ("chrome_console_messages", ToolBudgetClass::Read),
    ("chrome_network_requests", ToolBudgetClass::Read),
    ("chrome_screenshot", ToolBudgetClass::Read),
];

pub fn classify_tool_budget_class(tool_name: &str) -> ToolBudgetClass {
    let tool_name = tool_name.trim();
    TOOL_BUDGET_CLASS_BY_NAME
        .iter()
        .find_map(|(name, class)| (*name == tool_name).then_some(*class))
        .unwrap_or(ToolBudgetClass::Other)
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
    if let Some(window) = policy.post_mutation_verification_window {
        if observations.iter().any(observation_is_successful_mutation) {
            apply_post_mutation_verification_window(&mut next_state.tool_usage, window);
        }
    }

    let batch_signature = rule_of_ko_batch_signature(observations);
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
    } else if !observations.is_empty() {
        next_state.last_batch_signature = None;
        next_state.same_batch_signature_repeats = 0;
    }

    let batch_failed =
        !observations.is_empty() && observations.iter().all(|observation| observation.failed);
    if batch_failed {
        next_state.consecutive_tool_failures =
            prior_state.consecutive_tool_failures.saturating_add(1);
    } else if !observations.is_empty() {
        next_state.consecutive_tool_failures = 0;
    }

    let stop_candidate = if elapsed_ms >= policy.max_wall_clock_ms {
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

    let finalization_grace_stop = matches!(
        stop_candidate,
        Some(
            TurnBudgetStopReason::WallClockLimit { .. }
                | TurnBudgetStopReason::TotalToolCallLimit { .. }
                | TurnBudgetStopReason::ToolClassCallLimit { .. }
        )
    );
    let stop_reason = if finalization_grace_stop && !prior_state.budget_finalization_grace_used {
        next_state.budget_finalization_grace_used = true;
        None
    } else {
        stop_candidate.clone()
    };

    let warning = if stop_reason.is_none() {
        budget_warning(
            policy,
            step,
            elapsed_ms,
            &next_state,
            stop_candidate.as_ref(),
        )
    } else {
        None
    };

    TurnBudgetEvaluation {
        next_state,
        warning,
        stop_reason,
    }
}

pub fn evaluate_turn_context_budget(
    prior_state: &TurnBudgetState,
    report: ContextBudgetReport,
) -> TurnBudgetEvaluation {
    let mut next_state = prior_state.clone();
    next_state.latest_context_budget = Some(report.clone());

    let stop_reason = report
        .over_budget
        .then(|| TurnBudgetStopReason::ContextBudgetLimit {
            model: report.model.clone(),
            estimated_input_tokens: report.estimated_input_tokens,
            reserved_output_tokens: report.reserved_output_tokens,
            estimated_total_tokens: report.estimated_total_tokens,
            context_window: report.context_window,
        });
    let warning = (stop_reason.is_none() && report.near_budget).then(|| {
        let code = "context_budget_warning";
        TurnBudgetWarning {
            code,
            message: render_budget_warning_message(
                json!({
                    "code": code,
                    "model": report.model,
                    "context_window": report.context_window,
                    "estimated_input_tokens": report.estimated_input_tokens,
                    "reserved_output_tokens": report.reserved_output_tokens,
                    "estimated_total_tokens": report.estimated_total_tokens,
                }),
                "Budget advisory: the next model call is close to the context limit. Checkpoint or compact before adding more context. If focused work remains, split the remaining work into smaller task slices rather than trying to fit the whole Job in this turn."
                    .to_string(),
            ),
        }
    });

    TurnBudgetEvaluation {
        next_state,
        warning,
        stop_reason,
    }
}

fn render_budget_warning_message(budget: Value, fallback: String) -> String {
    // Keep reusable budget steering prose in the fragment registry. Rust should
    // only decide which warning applies and pass structured state to the
    // renderer so loop-control prompts stay auditable with other fragments.
    let rendered = repository_prompt_fragment_registry()
        .and_then(|fragments| fragments.require("runtime_budget_warning").cloned())
        .and_then(|fragment| render_turn_fragment(&fragment, &json!({ "budget": budget })));
    rendered.unwrap_or(fallback)
}

fn budget_warning(
    policy: TurnBudgetPolicy,
    step: u32,
    elapsed_ms: u64,
    state: &TurnBudgetState,
    finalization_stop: Option<&TurnBudgetStopReason>,
) -> Option<TurnBudgetWarning> {
    if let Some(reason) = finalization_stop {
        return Some(match reason {
            TurnBudgetStopReason::WallClockLimit {
                elapsed_ms,
                limit_ms,
            } => {
                let code = "wall_clock_finalization_warning";
                TurnBudgetWarning {
                    code,
                    message: render_budget_warning_message(
                        json!({
                            "code": code,
                            "elapsed_ms": elapsed_ms,
                            "limit_ms": limit_ms,
                        }),
                        format!(
                            "Budget advisory: this turn reached its wall-clock limit after recording the latest tool result (elapsed={elapsed_ms}ms/limit={limit_ms}ms). Do not call more tools in this turn. If focused work remains, summarize progress and prepare a smaller next task slice rather than claiming the Job is complete."
                        ),
                    ),
                }
            }
            TurnBudgetStopReason::TotalToolCallLimit { .. }
            | TurnBudgetStopReason::ToolClassCallLimit { .. } => {
                let code = "tool_budget_finalization_warning";
                TurnBudgetWarning {
                    code,
                    message: render_budget_warning_message(
                        json!({ "code": code }),
                        "Budget advisory: this turn has exceeded a tool budget after recording the latest tool result. Do not call more tools in this turn. If focused work remains, summarize progress and prepare a smaller next task slice rather than claiming the Job is complete.".to_string(),
                    ),
                }
            }
            _ => return None,
        });
    }

    if step + 1 >= policy.emergency_hard_steps {
        let code = "emergency_hard_step_warning";
        return Some(TurnBudgetWarning {
            code,
            message: render_budget_warning_message(
                json!({
                    "code": code,
                    "next_step": step + 1,
                    "limit": policy.emergency_hard_steps,
                }),
                format!(
                    "Budget advisory: this turn is at the end of its emergency continuation fuse (next step would reach {}/{}). If you already have enough information for this slice, stop calling tools, record progress, and leave a smaller next slice if focused work remains.",
                    step + 1,
                    policy.emergency_hard_steps
                ),
            ),
        });
    }

    let remaining_wall_clock_ms = policy.max_wall_clock_ms.saturating_sub(elapsed_ms);
    if remaining_wall_clock_ms <= 15_000
        || elapsed_ms.saturating_mul(100) >= policy.max_wall_clock_ms.saturating_mul(85)
    {
        let code = "wall_clock_warning";
        return Some(TurnBudgetWarning {
            code,
            message: render_budget_warning_message(
                json!({
                    "code": code,
                    "remaining_ms": remaining_wall_clock_ms,
                }),
                format!(
                    "Budget advisory: this turn is close to its wall-clock limit (remaining={}ms). Prefer finishing the current slice, checkpointing, or decomposing remaining focused work over starting another broad tool sequence. Use one more tool call only if it is strictly necessary to safely close this slice.",
                    remaining_wall_clock_ms
                ),
            ),
        });
    }

    if state.tool_usage.total >= policy.tool_call_limits.total {
        let code = "total_tool_budget_warning";
        let fragment_code = "total_tool_budget_full_warning";
        return Some(TurnBudgetWarning {
            code,
            message: render_budget_warning_message(
                json!({
                    "code": fragment_code,
                    "used": state.tool_usage.total,
                    "limit": policy.tool_call_limits.total,
                }),
                format!(
                    "Budget advisory: this turn has fully used its emergency total tool-call fuse ({}/{} tool calls used). Any further tool call will stop the turn. Finish/checkpoint this slice; if focused work remains, prepare a smaller next task slice instead of treating the budget as Job completion.",
                    state.tool_usage.total,
                    policy.tool_call_limits.total
                ),
            ),
        });
    }

    if state.tool_usage.total + 1 >= policy.tool_call_limits.total {
        let code = "total_tool_budget_warning";
        return Some(TurnBudgetWarning {
            code,
            message: render_budget_warning_message(
                json!({
                    "code": code,
                    "used": state.tool_usage.total,
                    "limit": policy.tool_call_limits.total,
                }),
                format!(
                    "Budget advisory: this turn is close to its emergency total tool-call fuse ({}/{} tool calls used). Prefer finishing/checkpointing this slice and decomposing remaining focused work over more tool calls unless one more call is strictly necessary.",
                    state.tool_usage.total,
                    policy.tool_call_limits.total
                ),
            ),
        });
    }

    for &class in ALL_BUDGET_CLASSES_BY_SEVERITY {
        let count = state.tool_usage.count_for(class);
        let limit = policy.tool_call_limits.limit_for(class);
        if count == 0 {
            continue;
        }
        if count >= limit {
            let code = "tool_class_budget_warning";
            let fragment_code = "tool_class_budget_full_warning";
            return Some(TurnBudgetWarning {
                code,
                message: render_budget_warning_message(
                    json!({
                        "code": fragment_code,
                        "class_label": class.label(),
                        "used": count,
                        "limit": limit,
                    }),
                    format!(
                        "Budget advisory: this turn has fully used its {} tool budget ({}/{} used). Any further {} call will stop the turn. Finish/checkpoint this slice; if focused work remains, prepare a smaller next task slice instead of treating the budget as Job completion.",
                        class.label(),
                        count,
                        limit,
                        class.label()
                    ),
                ),
            });
        }
        if count + 1 >= limit {
            let code = "tool_class_budget_warning";
            return Some(TurnBudgetWarning {
                code,
                message: render_budget_warning_message(
                    json!({
                        "code": code,
                        "class_label": class.label(),
                        "used": count,
                        "limit": limit,
                    }),
                    format!(
                        "Budget advisory: this turn is close to its {} tool budget ({}/{} used). Prefer finishing/checkpointing this slice and decomposing remaining focused work over another {} call unless one more call is strictly necessary.",
                        class.label(),
                        count,
                        limit,
                        class.label()
                    ),
                ),
            });
        }
    }

    if state.consecutive_tool_failures + 1 >= policy.max_consecutive_tool_failures {
        let code = "failure_budget_warning";
        return Some(TurnBudgetWarning {
            code,
            message: render_budget_warning_message(
                json!({
                    "code": code,
                    "failures": state.consecutive_tool_failures,
                    "limit": policy.max_consecutive_tool_failures,
                }),
                format!(
                    "Budget advisory: this turn is close to its repeated-failure limit ({}/{} consecutive failed tool batches). Do not retry the same failing action unless you have new evidence.",
                    state.consecutive_tool_failures,
                    policy.max_consecutive_tool_failures
                ),
            ),
        });
    }

    if state.same_batch_signature_repeats >= policy.max_same_tool_signature_repeats {
        let code = "rule_of_ko_warning";
        return Some(TurnBudgetWarning {
            code,
            message: render_budget_warning_message(
                json!({
                    "code": code,
                    "repeats": state.same_batch_signature_repeats,
                    "limit": policy.max_same_tool_signature_repeats,
                }),
                format!(
                    "Budget advisory: this turn is close to its loop-ko limit ({}/{} repeated tool batches). Do not repeat the same tool call pattern again; either answer now or choose a materially different next step.",
                    state.same_batch_signature_repeats,
                    policy.max_same_tool_signature_repeats
                ),
            ),
        });
    }

    None
}

fn first_exhausted_class(
    usage: ToolCallBudgetUsage,
    limits: ToolCallBudgetLimits,
) -> Option<(ToolBudgetClass, u32, u32)> {
    for &class in ALL_BUDGET_CLASSES_BY_SEVERITY {
        let used = usage.count_for(class);
        let limit = limits.limit_for(class);
        if used > limit {
            return Some((class, used, limit));
        }
    }
    None
}

fn rule_of_ko_batch_signature(observations: &[ToolContinuationObservation]) -> Option<String> {
    if observations.is_empty() || observations.iter().all(observation_is_stateless_read_probe) {
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

fn observation_is_stateless_read_probe(observation: &ToolContinuationObservation) -> bool {
    !observation.failed
        && observation.class == ToolBudgetClass::Read
        && matches!(observation.tool_name.as_str(), "git_status")
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
            post_mutation_verification_window: Some(PostMutationVerificationWindow {
                replenish_read: 4,
                replenish_search: 2,
            }),
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
            grounding_probe_signal: None,
        }
    }

    fn context_budget_report(near_budget: bool, over_budget: bool) -> ContextBudgetReport {
        ContextBudgetReport {
            model: "test-model".to_string(),
            context_window: Some(100),
            max_output_tokens: Some(10),
            reserved_output_tokens: 10,
            estimated_input_tokens: if over_budget { 95 } else { 85 },
            estimated_total_tokens: if over_budget { 105 } else { 95 },
            estimate_precision: den_protocol::ContextBudgetEstimatePrecision::Approximate,
            near_budget,
            over_budget,
            components: Vec::new(),
        }
    }

    #[test]
    fn context_budget_report_updates_turn_budget_state() {
        let evaluation =
            evaluate_turn_context_budget(&state(), context_budget_report(false, false));

        assert!(evaluation.stop_reason.is_none());
        assert!(evaluation.warning.is_none());
        assert!(evaluation.next_state.latest_context_budget.is_some());
    }

    #[test]
    fn near_context_budget_warns_without_stopping() {
        let evaluation = evaluate_turn_context_budget(&state(), context_budget_report(true, false));

        assert!(evaluation.stop_reason.is_none());
        assert_eq!(
            evaluation.warning.as_ref().map(|warning| warning.code),
            Some("context_budget_warning")
        );
    }

    #[test]
    fn over_context_budget_stops_before_model_call() {
        let evaluation = evaluate_turn_context_budget(&state(), context_budget_report(true, true));

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::ContextBudgetLimit { .. })
        ));
        assert!(evaluation.warning.is_none());
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
    fn repeated_git_status_does_not_trigger_rule_of_ko() {
        let mut prior = state();
        prior.last_batch_signature = Some(tool_signature("memory_read", r#"{"path":"a"}"#));
        prior.same_batch_signature_repeats = 2;

        let evaluation = evaluate_turn_budget(
            policy(),
            3,
            1_000,
            &prior,
            &[observation("git_status", r#"{}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert_eq!(evaluation.next_state.last_batch_signature, None);
        assert_eq!(evaluation.next_state.same_batch_signature_repeats, 0);
    }

    #[test]
    fn rule_of_ko_warns_after_the_last_safe_repeat_not_before_it() {
        let mut prior = state();
        prior.last_batch_signature = Some(tool_signature("memory_read", r#"{"path":"a"}"#));
        prior.same_batch_signature_repeats = 1;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert_eq!(evaluation.next_state.same_batch_signature_repeats, 2);
        assert_eq!(
            evaluation.warning.as_ref().map(|warning| warning.code),
            Some("rule_of_ko_warning")
        );
    }

    #[test]
    fn different_tool_signature_resets_rule_of_ko_warning() {
        let mut prior = state();
        prior.last_batch_signature = Some(tool_signature("memory_read", r#"{"path":"a"}"#));
        prior.same_batch_signature_repeats = 1;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("memory_read", r#"{"path":"b"}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert_eq!(evaluation.next_state.same_batch_signature_repeats, 1);
        assert_ne!(
            evaluation.warning.as_ref().map(|warning| warning.code),
            Some("rule_of_ko_warning")
        );
    }

    #[test]
    fn wall_clock_budget_gets_one_finalization_warning() {
        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            60_000,
            &state(),
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(evaluation.stop_reason.is_none());
        assert!(evaluation.next_state.budget_finalization_grace_used);
        assert_eq!(
            evaluation.warning.as_ref().map(|warning| warning.code),
            Some("wall_clock_finalization_warning")
        );
    }

    #[test]
    fn wall_clock_warning_steers_to_a_user_facing_checkpoint() {
        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            52_000,
            &state(),
            &[observation("memory_read", r#"{\"path\":\"a\"}"#, false)],
        );
        let warning = evaluation.warning.expect("wall-clock warning");

        assert_eq!(warning.code, "wall_clock_warning");
        assert!(warning
            .message
            .contains("Do not begin another broad tool sequence"));
        assert!(warning
            .message
            .contains("progress/checkpoint status for the user"));
    }

    #[test]
    fn budget_warning_mentions_docket_for_durable_work_tracking() {
        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            60_000,
            &state(),
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );
        let warning = evaluation.warning.expect("budget warning");

        assert!(warning
            .message
            .contains("Docket tasks support much higher durable work-tracking limits"));
        assert!(warning
            .message
            .contains("without implying autonomous execution"));
        assert!(warning.message.contains("requiring an explicit Job"));
    }

    #[test]
    fn wall_clock_budget_stops_after_finalization_grace_is_used() {
        let mut prior = state();
        prior.budget_finalization_grace_used = true;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            60_000,
            &prior,
            &[observation("memory_read", r#"{"path":"a"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::WallClockLimit { .. })
        ));
    }

    #[test]
    fn read_budget_exhaustion_gets_one_finalization_warning() {
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

        assert!(evaluation.stop_reason.is_none());
        assert!(evaluation.next_state.budget_finalization_grace_used);
        assert_eq!(
            evaluation.warning.as_ref().map(|warning| warning.code),
            Some("tool_budget_finalization_warning")
        );
    }

    #[test]
    fn total_tool_budget_stops_after_finalization_grace_is_used() {
        let mut prior = state();
        prior.tool_usage.total = 20;
        prior.budget_finalization_grace_used = true;

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
        assert!(evaluation.warning.as_ref().is_some_and(|warning| warning
            .message
            .contains("Any further tool call will stop the turn")));
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
    fn successful_mutation_replenishes_read_and_search_budget() {
        let mut prior = state();
        prior.tool_usage.read = 12;
        prior.tool_usage.search = 7;
        prior.tool_usage.total = 19;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("fs_edit_file", r#"{"path":"a"}"#, false)],
        );

        assert_eq!(evaluation.next_state.tool_usage.read, 8);
        assert_eq!(evaluation.next_state.tool_usage.search, 5);
        assert_eq!(evaluation.next_state.tool_usage.total, 20);
        assert!(evaluation.stop_reason.is_none());
    }

    #[test]
    fn failed_grounding_probe_blocks_mutation_replenishment() {
        let mut prior = state();
        prior.tool_usage.read = 12;
        prior.tool_usage.search = 7;
        prior.tool_usage.total = 19;
        let mut mutation = observation("fs_edit_file", r#"{"path":"a"}"#, false);
        mutation.grounding_probe_signal = Some(GroundingProbeSignalKind::Fail);

        let evaluation = evaluate_turn_budget(policy(), 2, 1_000, &prior, &[mutation]);

        assert_eq!(evaluation.next_state.tool_usage.read, 12);
        assert_eq!(evaluation.next_state.tool_usage.search, 7);
        assert_eq!(evaluation.next_state.tool_usage.total, 20);
        assert!(evaluation.stop_reason.is_none());
    }

    #[test]
    fn failed_mutation_does_not_replenish_exploration_budget() {
        let mut prior = state();
        prior.tool_usage.read = 12;
        prior.tool_usage.search = 7;
        prior.tool_usage.total = 19;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("fs_edit_file", r#"{"path":"a"}"#, true)],
        );

        assert_eq!(evaluation.next_state.tool_usage.read, 12);
        assert_eq!(evaluation.next_state.tool_usage.search, 7);
        assert!(evaluation.stop_reason.is_none());
        assert!(evaluation.warning.is_some());
    }

    #[test]
    fn successful_execute_does_not_replenish_exploration_budget() {
        let mut prior = state();
        prior.tool_usage.read = 12;
        prior.tool_usage.search = 7;
        prior.tool_usage.total = 19;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation(
                "terminal_run_command",
                r#"{"command":"ls"}"#,
                false,
            )],
        );

        assert_eq!(evaluation.next_state.tool_usage.read, 12);
        assert_eq!(evaluation.next_state.tool_usage.search, 7);
        assert_eq!(evaluation.next_state.tool_usage.total, 20);
        assert!(evaluation.stop_reason.is_none());
    }

    #[test]
    fn mutation_replenishment_does_not_reset_total_budget_fuse() {
        let mut prior = state();
        prior.tool_usage.read = 12;
        prior.tool_usage.total = 20;
        prior.budget_finalization_grace_used = true;

        let evaluation = evaluate_turn_budget(
            policy(),
            2,
            1_000,
            &prior,
            &[observation("fs_edit_file", r#"{"path":"a"}"#, false)],
        );

        assert!(matches!(
            evaluation.stop_reason,
            Some(TurnBudgetStopReason::TotalToolCallLimit {
                count: 21,
                limit: 20,
            })
        ));
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
