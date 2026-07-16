//! Internal ops/audit records for conversation lifecycle review.
//!
//! These types describe what Den observed while reviewing a conversation. They are
//! operational metadata, not transcript content, model context, or user-visible
//! messages.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationReview {
    pub id: Uuid,
    pub conversation_id: String,
    pub client_session_id: Option<String>,
    pub run_id: Option<Uuid>,
    pub trigger: ConversationReviewTrigger,
    pub findings: Vec<ConversationReviewFinding>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl ConversationReview {
    pub fn new(
        conversation_id: impl Into<String>,
        client_session_id: Option<String>,
        run_id: Option<Uuid>,
        trigger: ConversationReviewTrigger,
        findings: Vec<ConversationReviewFinding>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id: conversation_id.into(),
            client_session_id,
            run_id,
            trigger,
            findings,
            created_at: OffsetDateTime::now_utc(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationReviewTrigger {
    TurnEnd,
    SessionClose,
    ContextPressure,
    ToolError,
    Manual,
    OpenSessionSweep,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationReviewFinding {
    pub source: FindingSource,
    pub detail: ConversationReviewFindingDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FindingSource {
    pub detector: FindingDetector,
    pub confidence: f32,
    pub refs: Vec<String>,
}

impl FindingSource {
    pub fn runtime(refs: Vec<String>) -> Self {
        Self {
            detector: FindingDetector::Runtime,
            confidence: 1.0,
            refs,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingDetector {
    Runtime,
    Heuristic,
    ReviewModel,
    Human,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationReviewFindingDetail {
    CompactionNeeded {
        reason: String,
    },
    MemoryReflectionCandidate {
        reason: String,
    },
    MissedTaskCapture {
        reason: String,
    },
    UnnecessaryTaskCapture {
        reason: String,
    },
    UserFrustration {
        reason: String,
    },
    ErrorEncountered {
        reason: String,
        error_ref: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_non_compaction_finding_without_severity_or_action() {
        let review = ConversationReview {
            id: Uuid::nil(),
            conversation_id: "conv_123".to_string(),
            client_session_id: Some("session_123".to_string()),
            run_id: None,
            trigger: ConversationReviewTrigger::SessionClose,
            findings: vec![ConversationReviewFinding {
                source: FindingSource {
                    detector: FindingDetector::ReviewModel,
                    confidence: 0.8,
                    refs: vec!["turn_7".to_string()],
                },
                detail: ConversationReviewFindingDetail::MissedTaskCapture {
                    reason: "User asked for durable planning, but no Docket task was captured."
                        .to_string(),
                },
            }],
            created_at: OffsetDateTime::UNIX_EPOCH,
        };

        let value = serde_json::to_value(review).expect("review serializes");
        assert_eq!(value["trigger"], "session_close");
        assert_eq!(
            value["findings"][0]["detail"]["kind"],
            "missed_task_capture"
        );
        assert!(value["findings"][0].get("severity").is_none());
        assert!(value.get("recommended_actions").is_none());
    }
}
