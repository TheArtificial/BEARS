//! Minimal strategy policy selector ([ADR-0033](../../../docs/decisions/adr-0033-model-tasks-layer.md)).

use super::StrategyProfile;

#[derive(Debug, Clone, Copy)]
pub struct StrategyPolicyInput {
    pub task_kind: Option<&'static str>,
    pub has_command_criteria: bool,
    pub difficulty: Option<&'static str>,
}

pub fn select_strategy_profile(input: StrategyPolicyInput) -> StrategyProfile {
    if input.has_command_criteria {
        return StrategyProfile {
            plan: false,
            reflect_on_fail: true,
            critique: false,
            fanout_n: 0,
        };
    }
    if matches!(input.task_kind, Some("investigation")) {
        return StrategyProfile {
            plan: false,
            reflect_on_fail: false,
            critique: false,
            fanout_n: 2,
        };
    }
    if matches!(input.difficulty, Some("high") | Some("very_high")) {
        return StrategyProfile {
            plan: false,
            reflect_on_fail: false,
            critique: true,
            fanout_n: 0,
        };
    }
    StrategyProfile::plain_react()
}
