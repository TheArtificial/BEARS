//! Capability profiles for API-direct native roles ([Phase 5](../../../../docs/roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

use crate::agent_loop::{
    PostMutationVerificationWindow, StrategyProfile, ToolCallBudgetLimits, TurnBudgetPolicy,
};
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
    pub fn with_tool_budget_multiplier(mut self, multiplier: f64) -> Self {
        self.turn_budget = scaled_turn_budget(self.turn_budget, multiplier);
        self
    }

    pub fn for_profile(profile: BearProfile) -> Self {
        match profile {
            BearProfile::Pair => Self {
                profile,
                turn_budget: TurnBudgetPolicy {
                    max_wall_clock_ms: 360_000,
                    emergency_hard_steps: 80,
                    tool_call_limits: ToolCallBudgetLimits {
                        total: 112,
                        read: 32,
                        search: 20,
                        fetch: 12,
                        execute: 16,
                        write: 24,
                        destructive: 2,
                        other: 16,
                    },
                    max_consecutive_tool_failures: 3,
                    max_same_tool_signature_repeats: 2,
                    post_mutation_verification_window: Some(PostMutationVerificationWindow {
                        replenish_read: 4,
                        replenish_search: 2,
                    }),
                },
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Curate => Self {
                profile,
                turn_budget: TurnBudgetPolicy {
                    max_wall_clock_ms: 180_000,
                    emergency_hard_steps: 36,
                    tool_call_limits: ToolCallBudgetLimits {
                        total: 48,
                        read: 12,
                        search: 8,
                        fetch: 6,
                        execute: 4,
                        write: 8,
                        destructive: 1,
                        other: 8,
                    },
                    max_consecutive_tool_failures: 3,
                    max_same_tool_signature_repeats: 2,
                    post_mutation_verification_window: Some(PostMutationVerificationWindow {
                        replenish_read: 3,
                        replenish_search: 1,
                    }),
                },
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Watch => Self {
                profile,
                turn_budget: TurnBudgetPolicy {
                    max_wall_clock_ms: 90_000,
                    emergency_hard_steps: 16,
                    tool_call_limits: ToolCallBudgetLimits {
                        total: 16,
                        read: 8,
                        search: 4,
                        fetch: 3,
                        execute: 2,
                        write: 2,
                        destructive: 1,
                        other: 4,
                    },
                    max_consecutive_tool_failures: 2,
                    max_same_tool_signature_repeats: 1,
                    post_mutation_verification_window: None,
                },
                include_prompt_memory: false,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Chat | BearProfile::Work => Self {
                profile,
                turn_budget: if profile == BearProfile::Work {
                    TurnBudgetPolicy {
                        max_wall_clock_ms: 900_000,
                        emergency_hard_steps: 128,
                        tool_call_limits: ToolCallBudgetLimits {
                            total: 160,
                            read: 48,
                            search: 32,
                            fetch: 20,
                            execute: 24,
                            write: 24,
                            destructive: 6,
                            other: 24,
                        },
                        max_consecutive_tool_failures: 4,
                        max_same_tool_signature_repeats: 2,
                        post_mutation_verification_window: Some(PostMutationVerificationWindow {
                            replenish_read: 8,
                            replenish_search: 4,
                        }),
                    }
                } else {
                    TurnBudgetPolicy {
                        max_wall_clock_ms: 180_000,
                        emergency_hard_steps: 40,
                        tool_call_limits: ToolCallBudgetLimits {
                            total: 72,
                            read: 20,
                            search: 12,
                            fetch: 8,
                            execute: 6,
                            write: 12,
                            destructive: 2,
                            other: 12,
                        },
                        max_consecutive_tool_failures: 3,
                        max_same_tool_signature_repeats: 2,
                        post_mutation_verification_window: Some(PostMutationVerificationWindow {
                            replenish_read: 4,
                            replenish_search: 2,
                        }),
                    }
                },
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
        }
    }
}

fn scaled_count(value: u32, multiplier: f64) -> u32 {
    if !multiplier.is_finite() || multiplier <= 0.0 {
        return value;
    }
    (f64::from(value) * multiplier)
        .ceil()
        .clamp(1.0, f64::from(u32::MAX)) as u32
}

fn scaled_turn_budget(mut budget: TurnBudgetPolicy, multiplier: f64) -> TurnBudgetPolicy {
    if !multiplier.is_finite() || (multiplier - 1.0).abs() < f64::EPSILON {
        return budget;
    }
    budget.emergency_hard_steps = scaled_count(budget.emergency_hard_steps, multiplier);
    budget.tool_call_limits = ToolCallBudgetLimits {
        total: scaled_count(budget.tool_call_limits.total, multiplier),
        read: scaled_count(budget.tool_call_limits.read, multiplier),
        search: scaled_count(budget.tool_call_limits.search, multiplier),
        fetch: scaled_count(budget.tool_call_limits.fetch, multiplier),
        execute: scaled_count(budget.tool_call_limits.execute, multiplier),
        write: scaled_count(budget.tool_call_limits.write, multiplier),
        destructive: scaled_count(budget.tool_call_limits.destructive, multiplier),
        other: scaled_count(budget.tool_call_limits.other, multiplier),
    };
    if let Some(window) = budget.post_mutation_verification_window.as_mut() {
        window.replenish_read = scaled_count(window.replenish_read, multiplier);
        window.replenish_search = scaled_count(window.replenish_search, multiplier);
    }
    budget
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
                    .emergency_hard_steps
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
        assert!(work.emergency_hard_steps > pair.emergency_hard_steps);
        assert!(work.max_wall_clock_ms > pair.max_wall_clock_ms);
        assert!(work.tool_call_limits.total > pair.tool_call_limits.total);
    }

    #[test]
    fn pair_profile_has_room_for_productive_multi_file_edits() {
        let pair = NativeCapabilityProfile::for_profile(BearProfile::Pair).turn_budget;
        assert!(pair.tool_call_limits.write >= 24);
        assert!(pair.tool_call_limits.total >= 100);
    }

    #[test]
    fn tool_budget_multiplier_scales_profile_limits() {
        let base = NativeCapabilityProfile::for_profile(BearProfile::Pair).turn_budget;
        let scaled = NativeCapabilityProfile::for_profile(BearProfile::Pair)
            .with_tool_budget_multiplier(1.5)
            .turn_budget;
        assert_eq!(
            scaled.tool_call_limits.total,
            (f64::from(base.tool_call_limits.total) * 1.5).ceil() as u32
        );
        assert_eq!(
            scaled.emergency_hard_steps,
            (f64::from(base.emergency_hard_steps) * 1.5).ceil() as u32
        );
    }
}
