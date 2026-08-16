use crate::ThinkingEffort;
use serde::{Deserialize, Serialize};

/// Symbolic classification for a foreground agent-loop model call.
///
/// This is intentionally independent of provider model identifiers. A shared model-tasks
/// authority can later resolve model eligibility and routing from this metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPrimaryStep {
    OrdinaryTurn,
    Planning,
    TaskSelection,
    Execution,
    Checkpoint,
    PreRiskReview,
    Summarization,
    CheapProbe,
}

impl AgentPrimaryStep {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OrdinaryTurn => "ordinary_turn",
            Self::Planning => "planning",
            Self::TaskSelection => "task_selection",
            Self::Execution => "execution",
            Self::Checkpoint => "checkpoint",
            Self::PreRiskReview => "pre_risk_review",
            Self::Summarization => "summarization",
            Self::CheapProbe => "cheap_probe",
        }
    }
}

/// Provider-neutral request settings resolved from symbolic model-task metadata.
///
/// This does not select a raw provider model. Model selection remains with the existing
/// Bear/model-library resolver; transports may omit unsupported optional fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelRequestProfile {
    pub agent_primary_step: AgentPrimaryStep,
    pub thinking_effort: Option<ThinkingEffort>,
}

pub const fn resolve_agent_primary_request_profile(
    agent_primary_step: AgentPrimaryStep,
    checkpoint_thinking_effort: Option<ThinkingEffort>,
) -> ModelRequestProfile {
    ModelRequestProfile {
        agent_primary_step,
        thinking_effort: match agent_primary_step {
            AgentPrimaryStep::Checkpoint | AgentPrimaryStep::PreRiskReview => {
                checkpoint_thinking_effort
            }
            AgentPrimaryStep::OrdinaryTurn
            | AgentPrimaryStep::Planning
            | AgentPrimaryStep::TaskSelection
            | AgentPrimaryStep::Execution
            | AgentPrimaryStep::Summarization
            | AgentPrimaryStep::CheapProbe => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_checkpoint_and_pre_risk_steps_receive_reasoning_effort() {
        assert_eq!(
            resolve_agent_primary_request_profile(
                AgentPrimaryStep::Checkpoint,
                Some(ThinkingEffort::High),
            )
            .thinking_effort,
            Some(ThinkingEffort::High)
        );
        assert_eq!(
            resolve_agent_primary_request_profile(
                AgentPrimaryStep::PreRiskReview,
                Some(ThinkingEffort::Medium),
            )
            .thinking_effort,
            Some(ThinkingEffort::Medium)
        );
        assert_eq!(
            resolve_agent_primary_request_profile(
                AgentPrimaryStep::Execution,
                Some(ThinkingEffort::High),
            )
            .thinking_effort,
            None
        );
    }

    #[test]
    fn step_names_are_stable() {
        assert_eq!(AgentPrimaryStep::CheapProbe.as_str(), "cheap_probe");
    }
}
