//! Symbolic execution profiles for economical Docket dispatch.
//!
//! Profiles deliberately do not name providers or concrete models. The runtime
//! resolves them against the active conversation/stance configuration; fallback
//! preserves that existing resolution when task metadata is inconclusive.

use serde::{Deserialize, Serialize};

use crate::model::DocketTaskDifficulty;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionProfile {
    Economy,
    Balanced,
    Advanced,
}

impl ExecutionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Economy => "economy",
            Self::Balanced => "balanced",
            Self::Advanced => "advanced",
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::Economy => Some(Self::Balanced),
            Self::Balanced => Some(Self::Advanced),
            Self::Advanced => None,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "economy" => Some(Self::Economy),
            "balanced" => Some(Self::Balanced),
            "advanced" => Some(Self::Advanced),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileProvenance {
    TaskDifficulty,
    ConversationFallback,
    SupervisorEscalation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedExecutionProfile {
    pub profile: Option<ExecutionProfile>,
    pub provenance: ProfileProvenance,
}

impl ResolvedExecutionProfile {
    pub fn persisted_value(self) -> String {
        match self.profile {
            Some(profile) => format!("{}:{}", self.provenance.as_str(), profile.as_str()),
            None => self.provenance.as_str().to_string(),
        }
    }
}

impl ProfileProvenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskDifficulty => "task_difficulty",
            Self::ConversationFallback => "conversation_fallback",
            Self::SupervisorEscalation => "supervisor_escalation",
        }
    }
}

/// Select the cheapest profile that is appropriate for the task's declared
/// difficulty. Unknown or absent metadata retains the existing conversation /
/// stance model resolution rather than guessing a concrete model.
pub fn resolve_execution_profile(
    difficulty: Option<DocketTaskDifficulty>,
) -> ResolvedExecutionProfile {
    let profile = match difficulty {
        Some(DocketTaskDifficulty::Trivial) => Some(ExecutionProfile::Economy),
        Some(DocketTaskDifficulty::Moderate) => Some(ExecutionProfile::Balanced),
        Some(DocketTaskDifficulty::Hard) => Some(ExecutionProfile::Advanced),
        Some(DocketTaskDifficulty::Unknown) | None => None,
    };
    ResolvedExecutionProfile {
        profile,
        provenance: if profile.is_some() {
            ProfileProvenance::TaskDifficulty
        } else {
            ProfileProvenance::ConversationFallback
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_cheapest_appropriate_profile() {
        assert_eq!(
            resolve_execution_profile(Some(DocketTaskDifficulty::Trivial)).profile,
            Some(ExecutionProfile::Economy)
        );
        assert_eq!(
            resolve_execution_profile(Some(DocketTaskDifficulty::Moderate)).profile,
            Some(ExecutionProfile::Balanced)
        );
        assert_eq!(
            resolve_execution_profile(Some(DocketTaskDifficulty::Hard)).profile,
            Some(ExecutionProfile::Advanced)
        );
    }

    #[test]
    fn unknown_metadata_preserves_conversation_fallback() {
        let resolved = resolve_execution_profile(Some(DocketTaskDifficulty::Unknown));
        assert_eq!(resolved.profile, None);
        assert_eq!(resolved.provenance, ProfileProvenance::ConversationFallback);
        assert_eq!(resolved.persisted_value(), "conversation_fallback");
    }

    #[test]
    fn escalation_has_a_hard_ceiling() {
        assert_eq!(
            ExecutionProfile::Economy.next(),
            Some(ExecutionProfile::Balanced)
        );
        assert_eq!(
            ExecutionProfile::Balanced.next(),
            Some(ExecutionProfile::Advanced)
        );
        assert_eq!(ExecutionProfile::Advanced.next(), None);
    }
}
