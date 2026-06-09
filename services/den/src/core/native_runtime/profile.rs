//! Capability profiles for API-direct native roles ([Phase 5](../../../../docs/roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

use crate::core::{
    agent_loop::StrategyProfile,
    bears::BearProfile,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCapabilityProfile {
    pub role: BearProfile,
    pub max_steps: u32,
    /// When false, the turn relies on explicit runtime_context instead of prompt-memory blocks.
    pub include_prompt_memory: bool,
    pub strategy: StrategyProfile,
}

impl NativeCapabilityProfile {
    pub fn for_role(role: BearProfile) -> Self {
        match role {
            BearProfile::Pair => Self {
                role,
                max_steps: 8,
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Curate => Self {
                role,
                max_steps: 6,
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Watch => Self {
                role,
                max_steps: 4,
                include_prompt_memory: false,
                strategy: StrategyProfile::plain_react(),
            },
            BearProfile::Chat | BearProfile::Work => Self {
                role,
                max_steps: 8,
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
        for role in [
            BearProfile::Pair,
            BearProfile::Curate,
            BearProfile::Watch,
        ] {
            assert!(is_native_api_direct_role(role));
            assert!(NativeCapabilityProfile::for_role(role).max_steps > 0);
        }
    }

    #[test]
    fn harness_roles_are_not_api_direct() {
        assert!(!is_native_api_direct_role(BearProfile::Chat));
        assert!(!is_native_api_direct_role(BearProfile::Work));
    }
}
