//! Den-managed code execution sandboxes for `work`/`talk` harness ([ADR-0035](../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).
//!
//! Phase 7 spike: lifecycle hooks only; full container orchestration lands with harness rollout.

use uuid::Uuid;

use crate::errors::CustomError;

#[derive(Debug, Clone)]
pub struct SandboxSessionRef {
    pub session_id: String,
    pub bear_id: Uuid,
    pub workspace_root: String,
}

pub struct SandboxManager;

impl SandboxManager {
    pub fn new() -> Self {
        Self
    }

    pub async fn acquire_for_task(
        &self,
        bear_id: Uuid,
        task_id: &str,
    ) -> Result<SandboxSessionRef, CustomError> {
        Ok(SandboxSessionRef {
            session_id: format!("sbx-{bear_id}-{task_id}"),
            bear_id,
            workspace_root: format!("/tmp/bears-sbx/{bear_id}/{task_id}"),
        })
    }

    pub async fn release(&self, _session: &SandboxSessionRef) -> Result<(), CustomError> {
        Ok(())
    }
}

impl Default for SandboxManager {
    fn default() -> Self {
        Self::new()
    }
}
