use den_core::{AgentLoopControlLevel, ThinkingEffort};
use serde::{Deserialize, Serialize};

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
        profile
            .checkpoints
            .exploration_without_mutation_threshold,
    ) {
        Some(checkpoint_trigger(
            CheckpointReason::OverExploration,
            "Loop checkpoint: enough exploration has happened without a meaningful mutation; summarize evidence and choose the next action.",
        ))
    } else {
        None
    };

    if let Some(trigger) = trigger.as_ref() {
        next_state.last_checkpoint_reason = Some(trigger.reason);
    }

    CheckpointEvaluation {
        next_state,
        trigger,
    }
}

pub fn task_gate_checkpoint_trigger(profile: &AgentLoopControlProfile) -> Option<CheckpointTrigger> {
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
    } else if matches!(observation.class, ToolBudgetClass::Read | ToolBudgetClass::Search) {
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
                budget(240_000, 96, limits(128, 48, 32, 16, 20, 28, 3, 24), 3, 2, Some((6, 3))),
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
                budget(360_000, 80, limits(112, 32, 20, 12, 16, 24, 2, 16), 3, 2, Some((4, 2))),
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
                budget(360_000, 80, limits(96, 24, 16, 10, 14, 20, 2, 14), 2, 2, Some((4, 2))),
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
                budget(240_000, 64, limits(72, 16, 12, 8, 10, 14, 1, 10), 1, 1, Some((2, 1))),
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

    fn observation(class: ToolBudgetClass, signature: &str, failed: bool) -> ToolContinuationObservation {
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
        let low_budget = evaluate_checkpoint_trigger(
            &profile,
            &CheckpointState::default(),
            &[],
            true,
        );
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
