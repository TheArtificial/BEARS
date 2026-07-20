use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

#[derive(Clone, Default)]
pub(crate) struct ToolTaskRegistry {
    tasks: Arc<TokioMutex<HashMap<String, ToolTaskRecord>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ToolTaskRecord {
    pub(crate) session_id: String,
    pub(crate) tool_call_id: String,
    pub(crate) tool_name: String,
    pub(crate) turn_token: Option<Uuid>,
    pub(crate) phase: ToolTaskPhase,
    pub(crate) input_args: Option<Value>,
    pub(crate) started_at: std::time::Instant,
    pub(crate) updated_at: std::time::Instant,
}

impl ToolTaskRegistry {
    fn key(session_id: &str, tool_call_id: &str) -> String {
        format!("{session_id}\n{tool_call_id}")
    }

    pub(crate) async fn try_register(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        turn_token: Option<Uuid>,
    ) -> bool {
        let now = std::time::Instant::now();
        let mut tasks = self.tasks.lock().await;
        let key = Self::key(session_id, tool_call_id);
        if tasks.contains_key(&key) {
            return false;
        }
        tasks.insert(
            key,
            ToolTaskRecord {
                session_id: session_id.to_string(),
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                turn_token,
                phase: ToolTaskPhase::Received,
                input_args: None,
                started_at: now,
                updated_at: now,
            },
        );
        true
    }

    pub(crate) async fn remember_input(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        input_args: Value,
    ) {
        let mut tasks = self.tasks.lock().await;
        let Some(entry) = tasks.get_mut(&Self::key(session_id, tool_call_id)) else {
            return;
        };
        entry.tool_name = tool_name.to_string();
        entry.input_args = Some(input_args);
        entry.updated_at = std::time::Instant::now();
    }

    pub(crate) async fn get(&self, session_id: &str, tool_call_id: &str) -> Option<ToolTaskRecord> {
        self.tasks
            .lock()
            .await
            .get(&Self::key(session_id, tool_call_id))
            .cloned()
    }

    pub(crate) async fn set_phase(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        phase: ToolTaskPhase,
    ) {
        let mut tasks = self.tasks.lock().await;
        let now = std::time::Instant::now();
        let Some(entry) = tasks.get_mut(&Self::key(session_id, tool_call_id)) else {
            return;
        };
        let previous_phase = entry.phase;
        let previous_elapsed_ms = now.duration_since(entry.updated_at).as_millis();
        let total_elapsed_ms = now.duration_since(entry.started_at).as_millis();
        entry.phase = phase;
        entry.updated_at = now;
        if phase.should_log_to_stderr() || previous_phase.should_log_to_stderr() {
            eprintln!(
                "bear-armature: tool_task transition session_id={} tool_call_id={} tool_name={} from_phase={} to_phase={} phase_duration_ms={} total_duration_ms={}",
                session_id,
                tool_call_id,
                tool_name,
                previous_phase.as_str(),
                phase.as_str(),
                previous_elapsed_ms,
                total_elapsed_ms,
            );
        }
    }

    pub(crate) async fn remove(
        &self,
        session_id: &str,
        tool_call_id: &str,
    ) -> Option<ToolTaskRecord> {
        let removed = self
            .tasks
            .lock()
            .await
            .remove(&Self::key(session_id, tool_call_id));
        if let Some(record) = removed.as_ref() {
            if record.phase != ToolTaskPhase::ResultPosted {
                eprintln!(
                    "bear-armature: tool_task finished session_id={} tool_call_id={} tool_name={} final_phase={} total_duration_ms={}",
                    record.session_id,
                    record.tool_call_id,
                    record.tool_name,
                    record.phase.as_str(),
                    record.started_at.elapsed().as_millis(),
                );
            }
        }
        removed
    }

    pub(crate) async fn cancel_session(&self, session_id: &str) {
        self.cancel_matching(session_id, None).await;
    }

    async fn cancel_matching(&self, session_id: &str, turn_token: Option<Uuid>) {
        let mut tasks = self.tasks.lock().await;
        let now = std::time::Instant::now();
        tasks.retain(|_, task| {
            let matches = task.session_id == session_id
                && turn_token.is_none_or(|token| task.turn_token == Some(token));
            if !matches {
                return true;
            }
            if task.phase != ToolTaskPhase::ResultPosted {
                eprintln!(
                    "bear-armature: tool_task cancelled session_id={} turn_token={:?} tool_call_id={} tool_name={} from_phase={} total_duration_ms={}",
                    task.session_id,
                    task.turn_token,
                    task.tool_call_id,
                    task.tool_name,
                    task.phase.as_str(),
                    now.duration_since(task.started_at).as_millis(),
                );
            }
            false
        });
    }

    pub(crate) async fn list_for_session(&self, session_id: &str) -> Vec<ToolTaskRecord> {
        self.tasks
            .lock()
            .await
            .values()
            .filter(|task| task.session_id == session_id)
            .cloned()
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolTaskPhase {
    Received,
    PermissionRequested,
    PermissionGranted,
    PermissionDenied,
    PermissionTimeout,
    ExecutionStarted,
    ExecutionSucceeded,
    ExecutionFailed,
    ResultPosted,
    ResultPostFailed,
    Cancelled,
}

impl ToolTaskPhase {
    pub(crate) fn should_log_to_stderr(self) -> bool {
        matches!(
            self,
            Self::PermissionDenied
                | Self::PermissionTimeout
                | Self::ExecutionFailed
                | Self::ResultPostFailed
                | Self::Cancelled
        )
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::PermissionRequested => "permission_requested",
            Self::PermissionGranted => "permission_granted",
            Self::PermissionDenied => "permission_denied",
            Self::PermissionTimeout => "permission_timeout",
            Self::ExecutionStarted => "execution_started",
            Self::ExecutionSucceeded => "execution_succeeded",
            Self::ExecutionFailed => "execution_failed",
            Self::ResultPosted => "result_posted",
            Self::ResultPostFailed => "result_post_failed",
            Self::Cancelled => "cancelled",
        }
    }
}

pub(crate) fn log_tool_task_phase(
    session_id: &str,
    tool_call_id: &str,
    tool_name: &str,
    phase: ToolTaskPhase,
) {
    if !phase.should_log_to_stderr() {
        return;
    }
    eprintln!(
        "bear-armature: tool_task phase={} session_id={} tool_call_id={} tool_name={}",
        phase.as_str(),
        session_id,
        tool_call_id,
        tool_name
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_session_evicts_all_matching_task_records() {
        let registry = ToolTaskRegistry::default();
        let turn = Uuid::new_v4();
        assert!(
            registry
                .try_register("session-a", "call-a", "list_jobs", Some(turn))
                .await
        );
        assert!(
            registry
                .try_register("session-a", "call-b", "create_job", Some(Uuid::new_v4()))
                .await
        );
        assert!(
            registry
                .try_register("session-b", "call-c", "list_jobs", Some(turn))
                .await
        );

        registry.cancel_session("session-a").await;

        assert!(registry.list_for_session("session-a").await.is_empty());
        assert_eq!(registry.list_for_session("session-b").await.len(), 1);
    }

    #[tokio::test]
    async fn phase_and_input_updates_do_not_recreate_cancelled_records() {
        let registry = ToolTaskRegistry::default();
        let turn = Uuid::new_v4();
        assert!(
            registry
                .try_register("session-a", "call-a", "fs_read_text_file", Some(turn))
                .await
        );
        registry.cancel_session("session-a").await;

        registry
            .remember_input(
                "session-a",
                "call-a",
                "fs_read_text_file",
                serde_json::json!({"path":"README.md"}),
            )
            .await;
        registry
            .set_phase(
                "session-a",
                "call-a",
                "fs_read_text_file",
                ToolTaskPhase::ExecutionStarted,
            )
            .await;

        assert!(registry.list_for_session("session-a").await.is_empty());
    }

    #[tokio::test]
    async fn registry_tracks_phase_and_session_entries() {
        let registry = ToolTaskRegistry::default();
        assert!(
            registry
                .try_register("session-1", "call-1", "fs_list_directory", None)
                .await
        );
        registry
            .set_phase(
                "session-1",
                "call-1",
                "fs_list_directory",
                ToolTaskPhase::PermissionRequested,
            )
            .await;
        let items = registry.list_for_session("session-1").await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].phase, ToolTaskPhase::PermissionRequested);
        assert_eq!(items[0].tool_name, "fs_list_directory");
        assert!(items[0].updated_at >= items[0].started_at);
        let removed = registry.remove("session-1", "call-1").await.unwrap();
        assert_eq!(removed.tool_call_id, "call-1");
        assert!(registry.list_for_session("session-1").await.is_empty());
    }

    #[tokio::test]
    async fn try_register_rejects_duplicate_tool_call_for_session() {
        let registry = ToolTaskRegistry::default();

        assert!(
            registry
                .try_register("session-1", "call-1", "fs_read_text_file", None)
                .await
        );
        assert!(
            !registry
                .try_register("session-1", "call-1", "fs_read_text_file", None)
                .await
        );
        assert!(
            registry
                .try_register("session-1", "call-2", "fs_read_text_file", None)
                .await
        );
        assert!(
            registry
                .try_register("session-2", "call-1", "fs_read_text_file", None)
                .await
        );
    }
}
