use den_core::{AgentLoopControlLevel, ThinkingEffort};
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    PostMutationVerificationWindow, ToolBudgetClass, ToolCallBudgetLimits,
    ToolContinuationObservation, TurnBudgetPolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopControlSource {
    ModelDefault,
    BearOverride,
    StanceOverride,
    TaskEscalation,
    SystemDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReason {
    OverExploration,
    ConsecutiveFailure,
    SameSignatureNearKo,
    TaskGateRejection,
    LowBudget,
    PreRiskMutation,
}

impl CheckpointReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OverExploration => "over_exploration",
            Self::ConsecutiveFailure => "consecutive_failure",
            Self::SameSignatureNearKo => "same_signature_near_ko",
            Self::TaskGateRejection => "task_gate_rejection",
            Self::LowBudget => "low_budget",
            Self::PreRiskMutation => "pre_risk_mutation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KoPolicy {
    pub same_signature_warning_threshold: Option<u32>,
    pub max_same_tool_signature_repeats: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointPolicy {
    pub enabled: bool,
    pub exploration_without_mutation_threshold: Option<u32>,
    pub consecutive_failure_threshold: Option<u32>,
    pub same_signature_warning_threshold: Option<u32>,
    pub require_on_task_gate_rejection: bool,
    pub require_on_low_budget: bool,
    pub require_before_broad_mutation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskGatePolicy {
    pub checkpoint_on_first_rejection: bool,
    pub max_same_gate_rejections: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointThinkingPolicy {
    pub enabled: bool,
    pub checkpoint_turn_effort: Option<ThinkingEffort>,
    pub pre_risk_turn_effort: Option<ThinkingEffort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLoopControlProfile {
    pub budget: TurnBudgetPolicy,
    pub ko: KoPolicy,
    pub checkpoints: CheckpointPolicy,
    pub task_gate: TaskGatePolicy,
    pub thinking: CheckpointThinkingPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CheckpointState {
    pub read_search_since_mutation: u32,
    pub consecutive_failures: u32,
    pub same_signature_repeat_count: u32,
    pub last_signature: Option<String>,
    pub last_checkpoint_reason: Option<CheckpointReason>,
}

impl CheckpointState {
    /// Open a fresh checkpoint-observation window after the model has responded to a checkpoint.
    ///
    /// This intentionally resets only checkpoint trigger state. It does not replenish the
    /// authoritative turn-budget ledger or bypass rule-of-ko/failure hard stops.
    pub fn reset_after_checkpoint_report(&mut self) {
        self.read_search_since_mutation = 0;
        self.consecutive_failures = 0;
        self.same_signature_repeat_count = 0;
        self.last_signature = None;
        self.last_checkpoint_reason = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointTrigger {
    pub reason: CheckpointReason,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointEvaluation {
    pub next_state: CheckpointState,
    pub trigger: Option<CheckpointTrigger>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointField {
    ActiveObjective,
    Learned,
    RemainingUncertainty,
    MoreExplorationJustified,
    NextAction,
    TaskStateChangeNeeded,
    EvidenceRefs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCheckpointRequest {
    pub checkpoint_id: String,
    pub run_id: String,
    pub reason: CheckpointReason,
    pub control_level: AgentLoopControlLevel,
    pub active_objective: Option<String>,
    pub task_context: Option<CheckpointTaskContext>,
    pub evidence_refs: Vec<CheckpointEvidenceRef>,
    pub required_fields: Vec<CheckpointField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointTaskContext {
    pub task_list_id: Option<String>,
    pub task_list_version: Option<String>,
    pub active_item_id: Option<String>,
    pub active_item_title: Option<String>,
    pub docket_job_id: Option<String>,
    pub docket_task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointEvidenceRef {
    pub kind: String,
    pub id: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCheckpointResponse {
    pub checkpoint_id: String,
    pub active_objective: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub learned: Vec<String>,
    #[serde(default)]
    pub remaining_uncertainty: Vec<String>,
    pub more_exploration_justified: bool,
    pub next_action: CheckpointNextAction,
    #[serde(default)]
    pub task_state_change_needed: Option<TaskStateChangeIntent>,
    #[serde(default)]
    pub evidence_refs: Vec<CheckpointEvidenceRef>,
    #[serde(default)]
    pub confidence: Option<CheckpointConfidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointNextAction {
    CallTool { tool_name: Option<String> },
    Edit,
    Validate,
    UpdateTaskList,
    SyncTaskList,
    RequestHandoff,
    FinalIfGateAllows,
    StopBlocked,
}

impl<'de> Deserialize<'de> for CheckpointNextAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        checkpoint_next_action_from_value(value).map_err(serde::de::Error::custom)
    }
}

fn checkpoint_next_action_from_value(
    value: serde_json::Value,
) -> Result<CheckpointNextAction, String> {
    match value {
        serde_json::Value::String(value) => checkpoint_next_action_from_str(&value),
        serde_json::Value::Object(mut object) => {
            if let Some(tool_name) = object
                .remove("call_tool")
                .or_else(|| object.remove("tool_name"))
            {
                return Ok(CheckpointNextAction::CallTool {
                    tool_name: tool_name.as_str().map(str::to_string),
                });
            }
            let Some(action) = object
                .remove("action")
                .or_else(|| object.remove("type"))
                .and_then(|value| value.as_str().map(str::to_string))
            else {
                return Err(
                    "next_action object must include action/type or call_tool/tool_name"
                        .to_string(),
                );
            };
            let mut action = checkpoint_next_action_from_str(&action)?;
            if let CheckpointNextAction::CallTool { tool_name } = &mut action {
                if tool_name.is_none() {
                    *tool_name = object
                        .remove("tool_name")
                        .and_then(|value| value.as_str().map(str::to_string));
                }
            }
            Ok(action)
        }
        other => Err(format!(
            "next_action must be a string enum or object, got {other}"
        )),
    }
}

fn checkpoint_next_action_from_str(raw: &str) -> Result<CheckpointNextAction, String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let compact = normalized.replace(['-', ' '], "_");
    match compact.as_str() {
        "call_tool" | "tool" | "use_tool" => Ok(CheckpointNextAction::CallTool { tool_name: None }),
        "edit" => Ok(CheckpointNextAction::Edit),
        "validate" => Ok(CheckpointNextAction::Validate),
        "update_task_list" | "update_current_task_status" => Ok(CheckpointNextAction::UpdateTaskList),
        "sync_task_list" => Ok(CheckpointNextAction::SyncTaskList),
        "request_handoff" | "request_task_list_handoff" => Ok(CheckpointNextAction::RequestHandoff),
        "final_if_gate_allows" => Ok(CheckpointNextAction::FinalIfGateAllows),
        "stop_blocked" => Ok(CheckpointNextAction::StopBlocked),
        _ => classify_natural_language_checkpoint_action(&normalized).ok_or_else(|| {
            format!(
                "unknown next_action `{raw}`; expected one of call_tool, edit, validate, update_task_list, sync_task_list, request_handoff, final_if_gate_allows, stop_blocked"
            )
        }),
    }
}

fn classify_natural_language_checkpoint_action(text: &str) -> Option<CheckpointNextAction> {
    if text.contains("update_task")
        || text.contains("update task")
        || text.contains("task status")
        || text.contains("mark ")
    {
        return Some(CheckpointNextAction::UpdateTaskList);
    }
    if text.contains("sync_task") || text.contains("sync task") {
        return Some(CheckpointNextAction::SyncTaskList);
    }
    if text.contains("handoff") || text.contains("human review") || text.contains("escalat") {
        return Some(CheckpointNextAction::RequestHandoff);
    }
    if text.contains("test")
        || text.contains("validate")
        || text.contains("verify")
        || text.contains("check")
    {
        return Some(CheckpointNextAction::Validate);
    }
    if text.contains("edit")
        || text.contains("patch")
        || text.contains("mutat")
        || text.contains("change")
        || text.contains("introduce")
        || text.contains("implement")
    {
        return Some(CheckpointNextAction::Edit);
    }
    if text.contains("stop") || text.contains("blocked") {
        return Some(CheckpointNextAction::StopBlocked);
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStateChangeIntent {
    pub target_state: String,
    pub reason: String,
    pub evidence_refs: Vec<CheckpointEvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointResponseValidationError {
    CheckpointIdMismatch { expected: String, actual: String },
    MissingRequiredField(CheckpointField),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAgentLoopControl {
    pub level: AgentLoopControlLevel,
    pub source: AgentLoopControlSource,
    pub model_handle: Option<String>,
    pub profile: AgentLoopControlProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLoopControlResolutionInput<'a> {
    pub model_handle: Option<&'a str>,
    pub model_default: Option<AgentLoopControlLevel>,
    pub bear_override: Option<AgentLoopControlLevel>,
    pub stance_override: Option<AgentLoopControlLevel>,
    pub task_escalation: Option<AgentLoopControlLevel>,
}

pub fn resolve_agent_loop_control(
    input: AgentLoopControlResolutionInput<'_>,
) -> ResolvedAgentLoopControl {
    let (mut level, mut source) = if let Some(level) = input.model_default {
        (level, AgentLoopControlSource::ModelDefault)
    } else if let Some(model_handle) = input.model_handle {
        (
            den_llm::model_registry::default_agent_loop_control_for_model(model_handle),
            AgentLoopControlSource::ModelDefault,
        )
    } else {
        (
            AgentLoopControlLevel::default(),
            AgentLoopControlSource::SystemDefault,
        )
    };

    if let Some(override_level) = input.bear_override {
        level = override_level;
        source = AgentLoopControlSource::BearOverride;
    }
    if let Some(override_level) = input.stance_override {
        level = override_level;
        source = AgentLoopControlSource::StanceOverride;
    }
    if let Some(escalation) = input.task_escalation {
        let escalated = level.max(escalation);
        if escalated != level {
            level = escalated;
            source = AgentLoopControlSource::TaskEscalation;
        }
    }

    ResolvedAgentLoopControl {
        level,
        source,
        model_handle: input.model_handle.map(str::to_string),
        profile: AgentLoopControlProfile::for_level(level),
    }
}

pub fn evaluate_checkpoint_trigger(
    profile: &AgentLoopControlProfile,
    prior_state: &CheckpointState,
    observations: &[ToolContinuationObservation],
    low_budget: bool,
) -> CheckpointEvaluation {
    let mut next_state = prior_state.clone();
    for observation in observations {
        observe_checkpoint_tool_result(&mut next_state, observation);
    }

    let trigger = if !profile.checkpoints.enabled {
        None
    } else if profile.checkpoints.require_on_low_budget && low_budget {
        Some(checkpoint_trigger(
            CheckpointReason::LowBudget,
            "Loop checkpoint: remaining budget is low; synthesize what is known before continuing.",
        ))
    } else if threshold_reached(
        next_state.consecutive_failures,
        profile.checkpoints.consecutive_failure_threshold,
    ) {
        Some(checkpoint_trigger(
            CheckpointReason::ConsecutiveFailure,
            "Loop checkpoint: recent tool calls are failing; identify the failure pattern and choose a different recovery action.",
        ))
    } else if threshold_reached(
        next_state.same_signature_repeat_count,
        profile.checkpoints.same_signature_warning_threshold,
    ) {
        Some(checkpoint_trigger(
            CheckpointReason::SameSignatureNearKo,
            "Loop checkpoint: the same tool-call signature is repeating; choose a different action or explain why task state should change.",
        ))
    } else if threshold_reached(
        next_state.read_search_since_mutation,
        profile.checkpoints.exploration_without_mutation_threshold,
    ) {
        Some(checkpoint_trigger(
            CheckpointReason::OverExploration,
            "Loop checkpoint: enough exploration has happened without a meaningful mutation; summarize evidence and choose the next action.",
        ))
    } else {
        None
    };

    next_state.last_checkpoint_reason = trigger.as_ref().map(|trigger| trigger.reason);

    CheckpointEvaluation {
        next_state,
        trigger,
    }
}

pub fn validate_checkpoint_response(
    request: &RuntimeCheckpointRequest,
    response: &RuntimeCheckpointResponse,
) -> Result<(), CheckpointResponseValidationError> {
    if response.checkpoint_id != request.checkpoint_id {
        return Err(CheckpointResponseValidationError::CheckpointIdMismatch {
            expected: request.checkpoint_id.clone(),
            actual: response.checkpoint_id.clone(),
        });
    }

    for field in &request.required_fields {
        if checkpoint_field_missing(*field, response) {
            return Err(CheckpointResponseValidationError::MissingRequiredField(
                *field,
            ));
        }
    }

    Ok(())
}

fn checkpoint_field_missing(field: CheckpointField, response: &RuntimeCheckpointResponse) -> bool {
    match field {
        CheckpointField::ActiveObjective => response.active_objective.trim().is_empty(),
        CheckpointField::Learned => {
            response.learned.is_empty()
                && response
                    .summary
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
        }
        CheckpointField::RemainingUncertainty => response.remaining_uncertainty.is_empty(),
        CheckpointField::MoreExplorationJustified => false,
        CheckpointField::NextAction => false,
        CheckpointField::TaskStateChangeNeeded => response.task_state_change_needed.is_none(),
        CheckpointField::EvidenceRefs => response.evidence_refs.is_empty(),
    }
}

pub fn task_gate_checkpoint_trigger(
    profile: &AgentLoopControlProfile,
) -> Option<CheckpointTrigger> {
    profile
        .checkpoints
        .require_on_task_gate_rejection
        .then(|| checkpoint_trigger(
            CheckpointReason::TaskGateRejection,
            "Loop checkpoint: the final answer did not satisfy the active task gate; continue or update task state with evidence.",
        ))
}

pub fn pre_risk_checkpoint_trigger(profile: &AgentLoopControlProfile) -> Option<CheckpointTrigger> {
    profile
        .checkpoints
        .require_before_broad_mutation
        .then(|| checkpoint_trigger(
            CheckpointReason::PreRiskMutation,
            "Loop checkpoint: before a broad or risky mutation, state the evidence, intended change, and validation plan.",
        ))
}

fn observe_checkpoint_tool_result(
    state: &mut CheckpointState,
    observation: &ToolContinuationObservation,
) {
    if observation.failed {
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    } else {
        state.consecutive_failures = 0;
    }

    if state.last_signature.as_deref() == Some(observation.signature.as_str()) {
        state.same_signature_repeat_count = state.same_signature_repeat_count.saturating_add(1);
    } else {
        state.last_signature = Some(observation.signature.clone());
        state.same_signature_repeat_count = 0;
    }

    if observation_is_meaningful_mutation(observation) {
        state.read_search_since_mutation = 0;
    } else if matches!(
        observation.class,
        ToolBudgetClass::Read | ToolBudgetClass::Search
    ) {
        state.read_search_since_mutation = state.read_search_since_mutation.saturating_add(1);
    }
}

fn observation_is_meaningful_mutation(observation: &ToolContinuationObservation) -> bool {
    !observation.failed
        && matches!(
            observation.class,
            ToolBudgetClass::Write | ToolBudgetClass::Destructive
        )
}

fn threshold_reached(count: u32, threshold: Option<u32>) -> bool {
    threshold.is_some_and(|threshold| threshold > 0 && count >= threshold)
}

fn checkpoint_trigger(reason: CheckpointReason, message: &str) -> CheckpointTrigger {
    CheckpointTrigger {
        reason,
        message: message.to_string(),
    }
}

impl AgentLoopControlProfile {
    pub fn for_level(level: AgentLoopControlLevel) -> Self {
        match level {
            AgentLoopControlLevel::Light => Self::new(
                level,
                budget(
                    240_000,
                    96,
                    limits(128, 48, 32, 16, 20, 28, 3, 24),
                    3,
                    2,
                    Some((6, 3)),
                ),
                CheckpointPolicy {
                    enabled: true,
                    exploration_without_mutation_threshold: Some(8),
                    consecutive_failure_threshold: Some(2),
                    same_signature_warning_threshold: Some(2),
                    require_on_task_gate_rejection: false,
                    require_on_low_budget: true,
                    require_before_broad_mutation: false,
                },
                CheckpointThinkingPolicy {
                    enabled: false,
                    checkpoint_turn_effort: None,
                    pre_risk_turn_effort: None,
                },
            ),
            AgentLoopControlLevel::Standard => Self::new(
                level,
                budget(
                    360_000,
                    80,
                    limits(112, 32, 20, 12, 16, 24, 2, 16),
                    3,
                    2,
                    Some((4, 2)),
                ),
                CheckpointPolicy {
                    enabled: true,
                    exploration_without_mutation_threshold: Some(5),
                    consecutive_failure_threshold: Some(2),
                    same_signature_warning_threshold: Some(2),
                    require_on_task_gate_rejection: true,
                    require_on_low_budget: true,
                    require_before_broad_mutation: false,
                },
                CheckpointThinkingPolicy {
                    enabled: true,
                    checkpoint_turn_effort: Some(ThinkingEffort::Medium),
                    pre_risk_turn_effort: None,
                },
            ),
            AgentLoopControlLevel::Careful => Self::new(
                level,
                budget(
                    360_000,
                    80,
                    limits(96, 24, 16, 10, 14, 20, 2, 14),
                    2,
                    2,
                    Some((4, 2)),
                ),
                CheckpointPolicy {
                    enabled: true,
                    exploration_without_mutation_threshold: Some(3),
                    consecutive_failure_threshold: Some(1),
                    same_signature_warning_threshold: Some(1),
                    require_on_task_gate_rejection: true,
                    require_on_low_budget: true,
                    require_before_broad_mutation: true,
                },
                CheckpointThinkingPolicy {
                    enabled: true,
                    checkpoint_turn_effort: Some(ThinkingEffort::High),
                    pre_risk_turn_effort: Some(ThinkingEffort::High),
                },
            ),
            AgentLoopControlLevel::Strict => Self::new(
                level,
                budget(
                    240_000,
                    64,
                    limits(72, 16, 12, 8, 10, 14, 1, 10),
                    1,
                    1,
                    Some((2, 1)),
                ),
                CheckpointPolicy {
                    enabled: true,
                    exploration_without_mutation_threshold: Some(2),
                    consecutive_failure_threshold: Some(1),
                    same_signature_warning_threshold: Some(1),
                    require_on_task_gate_rejection: true,
                    require_on_low_budget: true,
                    require_before_broad_mutation: true,
                },
                CheckpointThinkingPolicy {
                    enabled: true,
                    checkpoint_turn_effort: Some(ThinkingEffort::High),
                    pre_risk_turn_effort: Some(ThinkingEffort::High),
                },
            ),
        }
    }

    pub fn with_budget(mut self, budget: TurnBudgetPolicy) -> Self {
        self.budget = budget;
        self.ko.max_same_tool_signature_repeats = budget.max_same_tool_signature_repeats;
        self
    }

    fn new(
        level: AgentLoopControlLevel,
        budget: TurnBudgetPolicy,
        checkpoints: CheckpointPolicy,
        thinking: CheckpointThinkingPolicy,
    ) -> Self {
        Self {
            budget,
            ko: KoPolicy {
                same_signature_warning_threshold: checkpoints.same_signature_warning_threshold,
                max_same_tool_signature_repeats: budget.max_same_tool_signature_repeats,
            },
            checkpoints,
            task_gate: TaskGatePolicy {
                checkpoint_on_first_rejection: !matches!(level, AgentLoopControlLevel::Light),
                max_same_gate_rejections: match level {
                    AgentLoopControlLevel::Light | AgentLoopControlLevel::Standard => 3,
                    AgentLoopControlLevel::Careful => 2,
                    AgentLoopControlLevel::Strict => 1,
                },
            },
            thinking,
        }
    }
}

fn limits(
    total: u32,
    read: u32,
    search: u32,
    fetch: u32,
    execute: u32,
    write: u32,
    destructive: u32,
    other: u32,
) -> ToolCallBudgetLimits {
    ToolCallBudgetLimits {
        total,
        read,
        search,
        fetch,
        execute,
        write,
        destructive,
        other,
    }
}

fn budget(
    max_wall_clock_ms: u64,
    emergency_hard_steps: u32,
    tool_call_limits: ToolCallBudgetLimits,
    max_consecutive_tool_failures: u32,
    max_same_tool_signature_repeats: u32,
    post_mutation_verification: Option<(u32, u32)>,
) -> TurnBudgetPolicy {
    TurnBudgetPolicy {
        max_wall_clock_ms,
        emergency_hard_steps,
        tool_call_limits,
        max_consecutive_tool_failures,
        max_same_tool_signature_repeats,
        post_mutation_verification_window: post_mutation_verification.map(
            |(replenish_read, replenish_search)| PostMutationVerificationWindow {
                replenish_read,
                replenish_search,
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        class: ToolBudgetClass,
        signature: &str,
        failed: bool,
    ) -> ToolContinuationObservation {
        ToolContinuationObservation {
            tool_name: class.label().to_string(),
            signature: signature.to_string(),
            class,
            failed,
        }
    }

    #[test]
    fn profiles_get_stricter_checkpoint_thresholds_by_level() {
        let light = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Light);
        let standard = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Standard);
        let careful = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Careful);
        let strict = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Strict);

        assert!(
            light.checkpoints.exploration_without_mutation_threshold
                > standard.checkpoints.exploration_without_mutation_threshold
        );
        assert!(
            standard.checkpoints.exploration_without_mutation_threshold
                > careful.checkpoints.exploration_without_mutation_threshold
        );
        assert!(
            careful.checkpoints.exploration_without_mutation_threshold
                >= strict.checkpoints.exploration_without_mutation_threshold
        );
        assert!(!light.checkpoints.require_before_broad_mutation);
        assert!(careful.checkpoints.require_before_broad_mutation);
        assert!(strict.checkpoints.require_before_broad_mutation);
    }

    #[test]
    fn checkpoint_thinking_policy_escalates_by_level() {
        let light = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Light);
        let standard = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Standard);
        let careful = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Careful);

        assert!(!light.thinking.enabled);
        assert_eq!(
            standard.thinking.checkpoint_turn_effort,
            Some(ThinkingEffort::Medium)
        );
        assert_eq!(
            careful.thinking.checkpoint_turn_effort,
            Some(ThinkingEffort::High)
        );
    }

    #[test]
    fn resolver_uses_model_default_then_overrides_then_escalation() {
        let resolved = resolve_agent_loop_control(AgentLoopControlResolutionInput {
            model_handle: Some("openai/gpt-5.5"),
            model_default: None,
            bear_override: None,
            stance_override: None,
            task_escalation: None,
        });
        assert_eq!(resolved.level, AgentLoopControlLevel::Light);
        assert_eq!(resolved.source, AgentLoopControlSource::ModelDefault);

        let resolved = resolve_agent_loop_control(AgentLoopControlResolutionInput {
            model_handle: Some("openai/gpt-5.5"),
            model_default: None,
            bear_override: Some(AgentLoopControlLevel::Standard),
            stance_override: Some(AgentLoopControlLevel::Careful),
            task_escalation: Some(AgentLoopControlLevel::Strict),
        });
        assert_eq!(resolved.level, AgentLoopControlLevel::Strict);
        assert_eq!(resolved.source, AgentLoopControlSource::TaskEscalation);
    }

    #[test]
    fn checkpoint_evaluator_triggers_on_over_exploration() {
        let profile = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Careful);
        let state = CheckpointState::default();
        let evaluation = evaluate_checkpoint_trigger(
            &profile,
            &state,
            &[
                observation(ToolBudgetClass::Read, "read:a", false),
                observation(ToolBudgetClass::Search, "search:b", false),
                observation(ToolBudgetClass::Read, "read:c", false),
            ],
            false,
        );

        assert_eq!(
            evaluation.trigger.as_ref().map(|trigger| trigger.reason),
            Some(CheckpointReason::OverExploration)
        );
        assert_eq!(evaluation.next_state.read_search_since_mutation, 3);
    }

    #[test]
    fn checkpoint_state_reset_after_report_opens_fresh_observation_window() {
        let mut state = CheckpointState {
            read_search_since_mutation: 7,
            consecutive_failures: 2,
            same_signature_repeat_count: 3,
            last_signature: Some("memory_read:{path=a}".to_string()),
            last_checkpoint_reason: Some(CheckpointReason::OverExploration),
        };

        state.reset_after_checkpoint_report();

        assert_eq!(state.read_search_since_mutation, 0);
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.same_signature_repeat_count, 0);
        assert_eq!(state.last_signature, None);
        assert_eq!(state.last_checkpoint_reason, None);
    }

    #[test]
    fn checkpoint_reset_allows_bounded_fresh_read_search_window() {
        let profile = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Careful);
        let mut state = CheckpointState {
            read_search_since_mutation: 3,
            last_checkpoint_reason: Some(CheckpointReason::OverExploration),
            ..CheckpointState::default()
        };
        state.reset_after_checkpoint_report();

        let evaluation = evaluate_checkpoint_trigger(
            &profile,
            &state,
            &[observation(ToolBudgetClass::Read, "read:a", false)],
            false,
        );

        assert!(evaluation.trigger.is_none());
        assert_eq!(evaluation.next_state.read_search_since_mutation, 1);
    }

    #[test]
    fn checkpoint_evaluator_clears_last_reason_when_no_trigger_fires() {
        let profile = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Standard);
        let state = CheckpointState {
            last_checkpoint_reason: Some(CheckpointReason::OverExploration),
            ..CheckpointState::default()
        };

        let evaluation = evaluate_checkpoint_trigger(
            &profile,
            &state,
            &[observation(ToolBudgetClass::Write, "write:a", false)],
            false,
        );

        assert!(evaluation.trigger.is_none());
        assert_eq!(evaluation.next_state.last_checkpoint_reason, None);
    }

    #[test]
    fn checkpoint_evaluator_resets_exploration_after_meaningful_mutation() {
        let profile = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Careful);
        let state = CheckpointState {
            read_search_since_mutation: 2,
            ..CheckpointState::default()
        };
        let evaluation = evaluate_checkpoint_trigger(
            &profile,
            &state,
            &[observation(ToolBudgetClass::Write, "write:a", false)],
            false,
        );

        assert!(evaluation.trigger.is_none());
        assert_eq!(evaluation.next_state.read_search_since_mutation, 0);
    }

    #[test]
    fn checkpoint_evaluator_triggers_on_failures_and_low_budget() {
        let profile = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Standard);
        let low_budget =
            evaluate_checkpoint_trigger(&profile, &CheckpointState::default(), &[], true);
        assert_eq!(
            low_budget.trigger.as_ref().map(|trigger| trigger.reason),
            Some(CheckpointReason::LowBudget)
        );

        let failures = evaluate_checkpoint_trigger(
            &profile,
            &CheckpointState::default(),
            &[
                observation(ToolBudgetClass::Read, "read:a", true),
                observation(ToolBudgetClass::Read, "read:b", true),
            ],
            false,
        );
        assert_eq!(
            failures.trigger.as_ref().map(|trigger| trigger.reason),
            Some(CheckpointReason::ConsecutiveFailure)
        );
    }

    #[test]
    fn task_gate_and_pre_risk_triggers_follow_profile_policy() {
        let light = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Light);
        let careful = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Careful);

        assert!(task_gate_checkpoint_trigger(&light).is_none());
        assert_eq!(
            task_gate_checkpoint_trigger(&careful).map(|trigger| trigger.reason),
            Some(CheckpointReason::TaskGateRejection)
        );
        assert!(pre_risk_checkpoint_trigger(&light).is_none());
        assert_eq!(
            pre_risk_checkpoint_trigger(&careful).map(|trigger| trigger.reason),
            Some(CheckpointReason::PreRiskMutation)
        );
    }

    fn checkpoint_request() -> RuntimeCheckpointRequest {
        RuntimeCheckpointRequest {
            checkpoint_id: "ckpt-1".to_string(),
            run_id: "run-1".to_string(),
            reason: CheckpointReason::OverExploration,
            control_level: AgentLoopControlLevel::Careful,
            active_objective: Some("Patch the failing path".to_string()),
            task_context: Some(CheckpointTaskContext {
                task_list_id: Some("list-1".to_string()),
                task_list_version: Some("3".to_string()),
                active_item_id: Some("item-1".to_string()),
                active_item_title: Some("Inspect routing".to_string()),
                docket_job_id: Some("job-1".to_string()),
                docket_task_id: Some("task-1".to_string()),
            }),
            evidence_refs: vec![CheckpointEvidenceRef {
                kind: "tool_result".to_string(),
                id: "call-1".to_string(),
                summary: Some("Read routing module".to_string()),
            }],
            required_fields: vec![
                CheckpointField::ActiveObjective,
                CheckpointField::Learned,
                CheckpointField::NextAction,
            ],
        }
    }

    fn checkpoint_response() -> RuntimeCheckpointResponse {
        RuntimeCheckpointResponse {
            checkpoint_id: "ckpt-1".to_string(),
            active_objective: "Patch the failing path".to_string(),
            summary: None,
            learned: vec!["The relevant logic is in the route projector.".to_string()],
            remaining_uncertainty: Vec::new(),
            more_exploration_justified: false,
            next_action: CheckpointNextAction::Edit,
            task_state_change_needed: None,
            evidence_refs: Vec::new(),
            confidence: Some(CheckpointConfidence::Medium),
        }
    }

    #[test]
    fn checkpoint_response_validation_accepts_structured_response() {
        let request = checkpoint_request();
        let response = checkpoint_response();

        assert_eq!(validate_checkpoint_response(&request, &response), Ok(()));
    }

    #[test]
    fn checkpoint_response_validation_rejects_wrong_id_and_missing_required_field() {
        let request = checkpoint_request();
        let mut response = checkpoint_response();
        response.checkpoint_id = "other".to_string();
        assert!(matches!(
            validate_checkpoint_response(&request, &response),
            Err(CheckpointResponseValidationError::CheckpointIdMismatch { .. })
        ));

        response.checkpoint_id = request.checkpoint_id.clone();
        response.learned.clear();
        assert_eq!(
            validate_checkpoint_response(&request, &response),
            Err(CheckpointResponseValidationError::MissingRequiredField(
                CheckpointField::Learned
            ))
        );
    }

    #[test]
    fn checkpoint_next_action_deserializes_natural_language_mutation() {
        let value = serde_json::json!(
            "Make the first meaningful mutation in den-runtime: introduce typed ToolCallWire payload structs."
        );
        let parsed: CheckpointNextAction = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, CheckpointNextAction::Edit);
    }

    #[test]
    fn checkpoint_next_action_deserializes_tool_object() {
        let value = serde_json::json!({
            "action": "call_tool",
            "tool_name": "fs_read_text_file"
        });
        let parsed: CheckpointNextAction = serde_json::from_value(value).unwrap();
        assert_eq!(
            parsed,
            CheckpointNextAction::CallTool {
                tool_name: Some("fs_read_text_file".to_string())
            }
        );
    }

    #[test]
    fn checkpoint_next_action_serializes_to_snake_case() {
        let serialized = serde_json::to_value(CheckpointNextAction::UpdateTaskList).unwrap();
        assert_eq!(serialized, serde_json::json!("update_task_list"));
    }

    #[test]
    fn profile_budget_can_be_overlaid_without_losing_level_policy() {
        let profile = AgentLoopControlProfile::for_level(AgentLoopControlLevel::Careful);
        let replacement_budget = budget(
            900_000,
            128,
            limits(160, 48, 32, 20, 24, 24, 6, 24),
            4,
            3,
            Some((8, 4)),
        );

        let overlaid = profile.with_budget(replacement_budget);

        assert_eq!(overlaid.budget, replacement_budget);
        assert_eq!(overlaid.ko.max_same_tool_signature_repeats, 3);
        assert_eq!(
            overlaid.checkpoints.exploration_without_mutation_threshold,
            Some(3)
        );
        assert_eq!(
            overlaid.thinking.checkpoint_turn_effort,
            Some(ThinkingEffort::High)
        );
    }

    #[test]
    fn escalation_never_downgrades_operator_override() {
        let resolved = resolve_agent_loop_control(AgentLoopControlResolutionInput {
            model_handle: Some("unknown-model"),
            model_default: None,
            bear_override: Some(AgentLoopControlLevel::Careful),
            stance_override: None,
            task_escalation: Some(AgentLoopControlLevel::Light),
        });
        assert_eq!(resolved.level, AgentLoopControlLevel::Careful);
        assert_eq!(resolved.source, AgentLoopControlSource::BearOverride);
    }
}
