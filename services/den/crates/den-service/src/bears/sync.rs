//! Native profile reconcile outcome types shared by provision and admin UI.

use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BearProfileSyncStatus {
    Synced,
    Failed,
    SkippedMissingBinding,
}

#[derive(Debug, Clone, Serialize)]
pub struct BearProfileSyncOutcome {
    pub profile: String,
    pub runtime_binding_id: Option<String>,
    pub status: BearProfileSyncStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BearSyncSummary {
    pub bear_id: Uuid,
    pub outcomes: Vec<BearProfileSyncOutcome>,
}

impl BearSyncSummary {
    pub fn failed_profiles(&self) -> Vec<&BearProfileSyncOutcome> {
        self.outcomes
            .iter()
            .filter(|o| o.status == BearProfileSyncStatus::Failed)
            .collect()
    }

    pub fn skipped_profiles(&self) -> Vec<&BearProfileSyncOutcome> {
        self.outcomes
            .iter()
            .filter(|o| o.status == BearProfileSyncStatus::SkippedMissingBinding)
            .collect()
    }

    pub fn synced_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == BearProfileSyncStatus::Synced)
            .count()
    }

    pub fn diagnostic_message(&self) -> Option<String> {
        let failed = self.failed_profiles();
        if failed.is_empty() {
            return None;
        }
        let parts = failed
            .into_iter()
            .map(|o| {
                format!(
                    "{} ({})",
                    o.profile,
                    o.message.as_deref().unwrap_or("unknown error")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        Some(format!("Profile reconcile failed for: {parts}"))
    }
}
