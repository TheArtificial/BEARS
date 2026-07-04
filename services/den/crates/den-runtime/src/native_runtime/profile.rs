//! Capability profiles for API-direct native roles ([Phase 5](../../../../docs/roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

use crate::agent_loop::{StrategyProfile, TurnBudgetPolicy};
use den_service::bears::BearProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCapabilityProfile {
    pub profile: BearProfile,
    pub turn_budget: TurnBudgetPolicy,
    /// When false, the turn relies on explicit runtime_context instead of prompt-memory blocks.
    pub include_prompt_memory: bool,
    pub strategy: StrategyProfile,
}

impl NativeCapabilityProfile {
    pub fn for_profile(profile: BearProfile) -> Self {
        match profile {
            BearProfile::Pair => Self {
                profile,
                turn_budget: TurnBudgetPolicy {
                    soft_steps: 6,
                    hard_steps: 12,
                    max_consecutive_tool_failures: 3,
                    max_same_tool_signature_repeats: 2,
                },
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Curate => Self {
                profile,
                turn_budget: TurnBudgetPolicy {
                    soft_steps: 5,
                    hard_steps: 10,
                    max_consecutive_tool_failures: 3,
                    max_same_tool_signature_repeats: 2,
                },
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Watch => Self {
                profile,
                turn_budget: TurnBudgetPolicy {
                    soft_steps: 3,
                    hard_steps: 6,
                    max_consecutive_tool_failures: 2,
                    max_same_tool_signature_repeats: 1,
                },
                include_prompt_memory: false,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Chat | BearProfile::Work => Self {
                profile,
                turn_budget: if profile == BearProfile::Work {
                    TurnBudgetPolicy {
                        soft_steps: 12,
                        hard_steps: 24,
                        max_consecutive_tool_failures: 4,
                        max_same_tool_signature_repeats: 2,
                    }
                } else {
                    TurnBudgetPolicy {
                        soft_steps: 6,
                        hard_steps: 10,
                        max_consecutive_tool_failures: 3,
                        max_same_tool_signature_repeats: 2,
                    }
                },
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
        }
    }
}

pub fn is_native_api_direct_role(role: BearProfile) -> bool {
    matches!(
        role,
        BearProfile::Pair | BearProfile::Curate | BearProfile::Watch
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_direct_roles_have_profiles() {
        for role in [BearProfile::Pair, BearProfile::Curate, BearProfile::Watch] {
            assert!(is_native_api_direct_role(role));
            assert!(
                NativeCapabilityProfile::for_profile(role)
                    .turn_budget
                    .hard_steps
                    > 0
            );
        }
    }

    #[test]
    fn harness_roles_are_not_api_direct() {
        assert!(!is_native_api_direct_role(BearProfile::Chat));
        assert!(!is_native_api_direct_role(BearProfile::Work));
    }

    #[test]
    fn work_profile_has_longer_total_budget_than_pair() {
        let pair = NativeCapabilityProfile::for_profile(BearProfile::Pair).turn_budget;
        let work = NativeCapabilityProfile::for_profile(BearProfile::Work).turn_budget;
        assert!(work.hard_steps > pair.hard_steps);
        assert!(work.soft_steps > pair.soft_steps);
    }
}
