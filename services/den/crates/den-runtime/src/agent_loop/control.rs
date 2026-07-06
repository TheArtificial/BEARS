use den_core::{AgentLoopControlLevel, ThinkingEffort};
use serde::{Deserialize, Serialize};

use super::{PostMutationVerificationWindow, ToolCallBudgetLimits, TurnBudgetPolicy};

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
