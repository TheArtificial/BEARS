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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRequestProfile {
    /// Canonical Den model handle approved by the existing Bear/model-library resolver.
    /// This is not a raw provider identifier and does not grant routing authority.
    pub approved_model_ref: String,
    pub agent_primary_step: AgentPrimaryStep,
    /// Catalog-authoritative support for optional reasoning metadata. Unknown is treated as
    /// unsupported so request construction never guesses from a provider or model name.
    pub supports_reasoning_effort: Option<bool>,
    pub thinking_effort: Option<ThinkingEffort>,
}

impl Default for ModelRequestProfile {
    fn default() -> Self {
        Self {
            approved_model_ref: String::new(),
            agent_primary_step: AgentPrimaryStep::OrdinaryTurn,
            supports_reasoning_effort: None,
            thinking_effort: None,
        }
    }
}

pub fn resolve_agent_primary_request_profile(
    approved_model_ref: impl Into<String>,
    agent_primary_step: AgentPrimaryStep,
    supports_reasoning_effort: Option<bool>,
    checkpoint_thinking_effort: Option<ThinkingEffort>,
) -> ModelRequestProfile {
    ModelRequestProfile {
        approved_model_ref: approved_model_ref.into(),
        agent_primary_step,
        supports_reasoning_effort,
        thinking_effort: matches!(supports_reasoning_effort, Some(true))
            .then_some(checkpoint_thinking_effort)
            .flatten()
            .filter(|_| {
                matches!(
                    agent_primary_step,
                    AgentPrimaryStep::Checkpoint | AgentPrimaryStep::PreRiskReview
                )
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_checkpoint_and_pre_risk_steps_receive_reasoning_effort() {
        assert_eq!(
            resolve_agent_primary_request_profile(
                "openai/gpt-5",
                AgentPrimaryStep::Checkpoint,
                Some(true),
                Some(ThinkingEffort::High),
            )
            .thinking_effort,
            Some(ThinkingEffort::High)
        );
        assert_eq!(
            resolve_agent_primary_request_profile(
                "openai/gpt-5",
                AgentPrimaryStep::PreRiskReview,
                Some(true),
                Some(ThinkingEffort::Medium),
            )
            .thinking_effort,
            Some(ThinkingEffort::Medium)
        );
        assert_eq!(
            resolve_agent_primary_request_profile(
                "openai/gpt-5",
                AgentPrimaryStep::Execution,
                Some(true),
                Some(ThinkingEffort::High),
            )
            .thinking_effort,
            None
        );
    }

    #[test]
    fn unsupported_or_unknown_capability_omits_reasoning_effort() {
        for support in [Some(false), None] {
            let profile = resolve_agent_primary_request_profile(
                "openai/gpt-5",
                AgentPrimaryStep::Checkpoint,
                support,
                Some(ThinkingEffort::High),
            );
            assert_eq!(profile.thinking_effort, None);
        }
    }

    #[test]
    fn resolved_profile_keeps_approved_model_reference() {
        let profile = resolve_agent_primary_request_profile(
            "openai/gpt-5",
            AgentPrimaryStep::OrdinaryTurn,
            None,
            None,
        );

        assert_eq!(profile.approved_model_ref, "openai/gpt-5");
    }

    #[test]
    fn step_names_are_stable() {
        assert_eq!(AgentPrimaryStep::CheapProbe.as_str(), "cheap_probe");
    }
}
