//! Background operation registry for long-running image work (pulls, builds).
//!
//! `docker pull`/`docker build` routinely outrun an HTTP request, so the
//! endpoints return an operation id immediately and the work streams into an
//! in-memory record with a byte-capped log tail. Operations live only in
//! provider memory: a restart forgets them (the underlying docker work may
//! still have completed — the image list is the durable truth).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::proc::{run_streaming, CommandSpec, TailBuffer};
use crate::protocol::{OperationDescriptor, OperationState};

pub const PULL_TIMEOUT: Duration = Duration::from_mins(15);
pub const BUILD_TIMEOUT: Duration = Duration::from_mins(30);
const OP_LOG_TAIL_BYTES: usize = 64 * 1024;
const MAX_FINISHED_OPS: usize = 50;

struct OpMutable {
    state: OperationState,
    error: Option<String>,
    finished_at: Option<OffsetDateTime>,
}

struct OpShared {
    kind: &'static str,
    target: String,
    started_at: OffsetDateTime,
    state: Mutex<OpMutable>,
    tail: Arc<Mutex<TailBuffer>>,
}

impl OpShared {
    fn descriptor(&self, id: &str) -> OperationDescriptor {
        let (state, error, finished_at) = {
            let mutable = self.state.lock().expect("op state lock");
            (mutable.state, mutable.error.clone(), mutable.finished_at)
        };
        let log_tail = self
            .tail
            .lock()
            .map(|tail| tail.snapshot_lossy())
            .unwrap_or_default();
        OperationDescriptor {
            id: id.to_string(),
            kind: self.kind.to_string(),
            target: self.target.clone(),
            state,
            log_tail,
            error,
            started_at: rfc3339(self.started_at),
            finished_at: finished_at.map(rfc3339),
        }
    }

    fn is_finished(&self) -> bool {
        self.state.lock().expect("op state lock").state != OperationState::Running
    }
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339)
        .unwrap_or_else(|_| "invalid-timestamp".to_string())
}

#[derive(Default)]
pub struct OpsRegistry {
    ops: Mutex<BTreeMap<String, Arc<OpShared>>>,
}

impl OpsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn `program args…` as a tracked background operation and return
    /// its id immediately. Output streams into the operation's log tail; the
    /// command is killed at `timeout`.
    pub fn spawn(
        self: &Arc<Self>,
        kind: &'static str,
        target: String,
        program: String,
        args: Vec<String>,
        timeout: Duration,
    ) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let shared = Arc::new(OpShared {
            kind,
            target,
            started_at: OffsetDateTime::now_utc(),
            state: Mutex::new(OpMutable {
                state: OperationState::Running,
                error: None,
                finished_at: None,
            }),
            tail: Arc::new(Mutex::new(TailBuffer::new(OP_LOG_TAIL_BYTES))),
        });
        {
            let mut ops = self.ops.lock().expect("ops lock");
            prune_finished(&mut ops);
            ops.insert(id.clone(), shared.clone());
        }

        tokio::spawn(async move {
            let mut spec = CommandSpec::new(&program, &args);
            spec.timeout = timeout;
            let result = run_streaming(spec, shared.tail.clone()).await;
            let mut mutable = shared.state.lock().expect("op state lock");
            mutable.finished_at = Some(OffsetDateTime::now_utc());
            match result {
                Ok(outcome) if outcome.success() => {
                    mutable.state = OperationState::Succeeded;
                }
                Ok(outcome) if outcome.timed_out => {
                    mutable.state = OperationState::Failed;
                    mutable.error = Some(format!("timed out after {}s", timeout.as_secs()));
                }
                Ok(outcome) => {
                    mutable.state = OperationState::Failed;
                    mutable.error = Some(format!(
                        "exited with status {}",
                        outcome
                            .exit_code
                            .map_or_else(|| "unknown".to_string(), |code| code.to_string())
                    ));
                }
                Err(err) => {
                    mutable.state = OperationState::Failed;
                    mutable.error = Some(err.to_string());
                }
            }
        });
        id
    }

    pub fn get(&self, id: &str) -> Option<OperationDescriptor> {
        self.ops
            .lock()
            .expect("ops lock")
            .get(id)
            .map(|shared| shared.descriptor(id))
    }

    /// All tracked operations, newest first.
    pub fn list(&self) -> Vec<OperationDescriptor> {
        let mut ops: Vec<OperationDescriptor> = self
            .ops
            .lock()
            .expect("ops lock")
            .iter()
            .map(|(id, shared)| shared.descriptor(id))
            .collect();
        ops.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        ops
    }
}

/// Drop the oldest finished operations beyond the retention cap. Running
/// operations are never pruned.
fn prune_finished(ops: &mut BTreeMap<String, Arc<OpShared>>) {
    let mut finished: Vec<(OffsetDateTime, String)> = ops
        .iter()
        .filter(|(_, shared)| shared.is_finished())
        .map(|(id, shared)| (shared.started_at, id.clone()))
        .collect();
    if finished.len() < MAX_FINISHED_OPS {
        return;
    }
    finished.sort_by_key(|entry| entry.0);
    let excess = finished.len() + 1 - MAX_FINISHED_OPS;
    for (_, id) in finished.into_iter().take(excess) {
        ops.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_terminal(registry: &Arc<OpsRegistry>, id: &str) -> OperationDescriptor {
        for _ in 0..200 {
            let descriptor = registry.get(id).expect("operation exists");
            if descriptor.state != OperationState::Running {
                return descriptor;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("operation {id} never finished");
    }

    #[tokio::test]
    async fn success_and_failure_flow_into_state_and_tail() {
        let registry = Arc::new(OpsRegistry::new());
        let ok = registry.spawn(
            "pull",
            "demo:latest".into(),
            "sh".into(),
            vec!["-c".into(), "printf 'pulled fine'".into()],
            Duration::from_secs(10),
        );
        let bad = registry.spawn(
            "build",
            "demo-build".into(),
            "sh".into(),
            vec!["-c".into(), "printf 'boom' >&2; exit 3".into()],
            Duration::from_secs(10),
        );

        let ok = wait_terminal(&registry, &ok).await;
        assert_eq!(ok.state, OperationState::Succeeded);
        assert!(ok.log_tail.contains("pulled fine"));
        assert!(ok.finished_at.is_some());

        let bad = wait_terminal(&registry, &bad).await;
        assert_eq!(bad.state, OperationState::Failed);
        assert!(bad.log_tail.contains("boom"));
        assert_eq!(bad.error.as_deref(), Some("exited with status 3"));

        // list() is newest-first and contains both.
        assert_eq!(registry.list().len(), 2);
    }

    #[tokio::test]
    async fn timeout_is_reported() {
        let registry = Arc::new(OpsRegistry::new());
        let id = registry.spawn(
            "pull",
            "slow:latest".into(),
            "sleep".into(),
            vec!["5".into()],
            Duration::from_millis(100),
        );
        let descriptor = wait_terminal(&registry, &id).await;
        assert_eq!(descriptor.state, OperationState::Failed);
        assert!(descriptor
            .error
            .as_deref()
            .unwrap_or("")
            .contains("timed out"));
    }

    #[tokio::test]
    async fn finished_ops_are_pruned_but_running_kept() {
        let registry = Arc::new(OpsRegistry::new());
        let mut ids = Vec::new();
        for index in 0..MAX_FINISHED_OPS {
            ids.push(registry.spawn(
                "pull",
                format!("img-{index}"),
                "true".into(),
                vec![],
                Duration::from_secs(5),
            ));
        }
        for id in &ids {
            wait_terminal(&registry, id).await;
        }
        // Next spawn prunes the oldest finished ones down to the cap.
        let running = registry.spawn(
            "build",
            "long".into(),
            "sleep".into(),
            vec!["30".into()],
            Duration::from_mins(1),
        );
        let ops = registry.list();
        assert!(ops.len() <= MAX_FINISHED_OPS);
        assert!(registry.get(&running).is_some(), "running op is kept");
    }
}
