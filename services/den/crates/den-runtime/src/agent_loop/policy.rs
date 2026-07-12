//! Minimal strategy policy selector ([ADR-0033](../../../docs/decisions/adr-0033-model-tasks-layer.md)).

use std::str::FromStr;

use super::StrategyProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyTaskKind {
    Investigation,
}

impl FromStr for StrategyTaskKind {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim() {
            "investigation" => Ok(Self::Investigation),
            other => Err(format!("unsupported strategy task kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyDifficulty {
    High,
    VeryHigh,
}

impl FromStr for StrategyDifficulty {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim() {
            "high" => Ok(Self::High),
            "very_high" => Ok(Self::VeryHigh),
            other => Err(format!("unsupported strategy difficulty: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StrategyPolicyInput {
    pub task_kind: Option<StrategyTaskKind>,
    pub has_command_criteria: bool,
    pub difficulty: Option<StrategyDifficulty>,
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
    if matches!(input.task_kind, Some(StrategyTaskKind::Investigation)) {
        return StrategyProfile {
            plan: false,
            reflect_on_fail: false,
            critique: false,
            fanout_n: 2,
        };
    }
    if matches!(
        input.difficulty,
        Some(StrategyDifficulty::High | StrategyDifficulty::VeryHigh)
    ) {
        return StrategyProfile {
            plan: false,
            reflect_on_fail: false,
            critique: true,
            fanout_n: 0,
        };
    }
    StrategyProfile::plain_react()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_criteria_prefers_reflection_over_fanout() {
        let profile = select_strategy_profile(StrategyPolicyInput {
            task_kind: Some(StrategyTaskKind::Investigation),
            has_command_criteria: true,
            difficulty: Some(StrategyDifficulty::High),
        });

        assert!(profile.reflect_on_fail);
        assert!(!profile.critique);
        assert_eq!(profile.fanout_n, 0);
    }

    #[test]
    fn investigation_uses_two_way_fanout() {
        let profile = select_strategy_profile(StrategyPolicyInput {
            task_kind: Some(StrategyTaskKind::Investigation),
            has_command_criteria: false,
            difficulty: None,
        });

        assert_eq!(profile.fanout_n, 2);
        assert!(!profile.critique);
    }

    #[test]
    fn high_difficulty_uses_critique() {
        let profile = select_strategy_profile(StrategyPolicyInput {
            task_kind: None,
            has_command_criteria: false,
            difficulty: Some(StrategyDifficulty::VeryHigh),
        });

        assert!(profile.critique);
        assert_eq!(profile.fanout_n, 0);
    }

    #[test]
    fn parses_typed_policy_hints() {
        assert_eq!(
            "investigation".parse::<StrategyTaskKind>(),
            Ok(StrategyTaskKind::Investigation)
        );
        assert_eq!(
            "very_high".parse::<StrategyDifficulty>(),
            Ok(StrategyDifficulty::VeryHigh)
        );
        assert!("typo".parse::<StrategyTaskKind>().is_err());
    }
}
