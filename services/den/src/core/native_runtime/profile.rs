//! Capability profiles for API-direct native roles ([Phase 5](../../../../docs/roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

use crate::core::{
    agent_loop::StrategyProfile,
    bears::BearAgentRole,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCapabilityProfile {
    pub role: BearAgentRole,
    pub max_steps: u32,
    /// When false, the turn relies on explicit runtime_context instead of prompt-memory blocks.
    pub include_prompt_memory: bool,
    pub strategy: StrategyProfile,
}

impl NativeCapabilityProfile {
    pub fn for_role(role: BearAgentRole) -> Self {
        match role {
            BearAgentRole::Pair => Self {
                role,
                max_steps: 8,
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
            BearAgentRole::Curate => Self {
                role,
                max_steps: 6,
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
            BearAgentRole::Watch => Self {
                role,
                max_steps: 4,
                include_prompt_memory: false,
                strategy: StrategyProfile::plain_react(),
            },
            BearAgentRole::Chat | BearAgentRole::Work => Self {
                role,
                max_steps: 8,
                include_prompt_memory: true,
                strategy: StrategyProfile::plain_react(),
            },
        }
    }
}

pub fn is_native_api_direct_role(role: BearAgentRole) -> bool {
    matches!(
        role,
        BearAgentRole::Pair | BearAgentRole::Curate | BearAgentRole::Watch
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_direct_roles_have_profiles() {
        for role in [
            BearAgentRole::Pair,
            BearAgentRole::Curate,
            BearAgentRole::Watch,
        ] {
            assert!(is_native_api_direct_role(role));
            assert!(NativeCapabilityProfile::for_role(role).max_steps > 0);
        }
    }

    #[test]
    fn harness_roles_are_not_api_direct() {
        assert!(!is_native_api_direct_role(BearAgentRole::Chat));
        assert!(!is_native_api_direct_role(BearAgentRole::Work));
    }
}
