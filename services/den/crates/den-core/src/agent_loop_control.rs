use serde::{Deserialize, Serialize};

/// Progressive runtime supervision level for tool-using agent loops.
///
/// The level is resolved before a run/turn and expands to a concrete runtime profile in
/// `den-runtime`. Keep this shared enum in `den-core` so the model registry, service config,
/// and runtime can agree on serialized values without dependency cycles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopControlLevel {
    Light,
    #[default]
    Standard,
    Careful,
    Strict,
}

impl AgentLoopControlLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Standard => "standard",
            Self::Careful => "careful",
            Self::Strict => "strict",
        }
    }

    /// Escalate to the stricter of two levels.
    pub fn max(self, other: Self) -> Self {
        if self >= other {
            self
        } else {
            other
        }
    }
}

/// Optional provider/model reasoning effort for checkpoint/pre-risk turns.
///
/// This is request metadata only. Runtime budget, ko, and task-gate enforcement must not depend
/// on providers honoring it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingEffort {
    Low,
    Medium,
    High,
}

impl ThinkingEffort {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_levels_have_monotonic_ordering() {
        assert!(AgentLoopControlLevel::Light < AgentLoopControlLevel::Standard);
        assert!(AgentLoopControlLevel::Standard < AgentLoopControlLevel::Careful);
        assert!(AgentLoopControlLevel::Careful < AgentLoopControlLevel::Strict);
        assert_eq!(
            AgentLoopControlLevel::Light.max(AgentLoopControlLevel::Careful),
            AgentLoopControlLevel::Careful
        );
    }

    #[test]
    fn serialized_values_are_stable_snake_case() {
        assert_eq!(
            serde_json::to_string(&AgentLoopControlLevel::Careful).unwrap(),
            "\"careful\""
        );
        assert_eq!(
            serde_json::to_string(&ThinkingEffort::High).unwrap(),
            "\"high\""
        );
    }
}
