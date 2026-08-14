use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use uuid::Uuid;

/// Serializes ACP projections that originate from one BearWire session.
///
/// A live event page owns the exclusive gate from before its fetch until all of
/// its frames have been projected. Detached local tools continue executing,
/// but must acquire the shared gate before they can emit an ACP update. This
/// prevents a fast tool task from overtaking lifecycle output already present
/// in a fetched (or in-flight) BearWire event page.
///
/// This is the ACP implementation of the BearWire ordered-surface-projection
/// invariant (ADR-0007 amendment). The JSON-RPC stdout write lock protects a
/// single JSON frame from interleaving, but it cannot establish semantic event
/// order between concurrent producers. Every BearWire-derived `session/update`
/// path must therefore acquire this dispatcher; detached local work may settle
/// callbacks concurrently but must not bypass canonical event-page projection.
#[derive(Clone, Default)]
pub(crate) struct AcpProjectionDispatcher {
    sessions: Arc<Mutex<HashMap<String, Arc<SessionProjectionGate>>>>,
}

struct SessionProjectionGate {
    page: Arc<RwLock<()>>,
    emission: Arc<Mutex<()>>,
}

impl Default for SessionProjectionGate {
    fn default() -> Self {
        Self {
            page: Arc::new(RwLock::new(())),
            emission: Arc::new(Mutex::new(())),
        }
    }
}

pub(crate) struct LiveProjectionPageGuard {
    _page: OwnedRwLockWriteGuard<()>,
    _emission: OwnedMutexGuard<()>,
    session_id: String,
    turn_token: Uuid,
}

pub(crate) struct DetachedProjectionGuard {
    _page: OwnedRwLockReadGuard<()>,
    _emission: OwnedMutexGuard<()>,
}

impl AcpProjectionDispatcher {
    async fn gate(&self, session_id: &str) -> Arc<SessionProjectionGate> {
        let mut sessions = self.sessions.lock().await;
        sessions
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionProjectionGate::default()))
            .clone()
    }

    /// Begin an ordered live BearWire event-page projection before fetching the
    /// page, so detached output cannot race a page once polling starts.
    pub(crate) async fn begin_live_page(
        &self,
        session_id: &str,
        turn_token: Uuid,
    ) -> LiveProjectionPageGuard {
        let gate = self.gate(session_id).await;
        let page = gate.page.clone().write_owned().await;
        let emission = gate.emission.clone().lock_owned().await;
        LiveProjectionPageGuard {
            _page: page,
            _emission: emission,
            session_id: session_id.to_string(),
            turn_token,
        }
    }

    /// Reserve an ordered ACP emission for a detached local tool task.
    pub(crate) async fn begin_detached_projection(
        &self,
        session_id: &str,
        _turn_token: Uuid,
    ) -> DetachedProjectionGuard {
        let gate = self.gate(session_id).await;
        let page = gate.page.clone().read_owned().await;
        let emission = gate.emission.clone().lock_owned().await;
        DetachedProjectionGuard {
            _page: page,
            _emission: emission,
        }
    }
}

impl LiveProjectionPageGuard {
    /// Records the server-owned sequence that is being projected while this
    /// page owns the session dispatcher. `None` is only used for canonical
    /// run-state recovery, which has no event-page frame sequence.
    pub(crate) fn observe_frame(&self, sequence: Option<i64>) {
        tracing::trace!(
            target: "bear_armature::lifecycle",
            session_id = self.session_id,
            turn_token = %self.turn_token,
            bearwire_sequence = ?sequence,
            "projecting ordered BearWire frame"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::{
        sync::Notify,
        time::{timeout, Duration},
    };

    #[tokio::test]
    async fn detached_projection_follows_the_entire_live_event_page_in_sequence_order() {
        let dispatcher = AcpProjectionDispatcher::default();
        let projected = Arc::new(Mutex::new(Vec::new()));
        let live = dispatcher
            .begin_live_page("session-a", Uuid::new_v4())
            .await;
        live.observe_frame(Some(41));
        projected.lock().await.push("frame-41");

        let ready = Arc::new(Notify::new());
        let completed = Arc::new(Notify::new());
        let task_dispatcher = dispatcher.clone();
        let task_projected = projected.clone();
        let task_ready = ready.clone();
        let task_completed = completed.clone();
        let task = tokio::spawn(async move {
            task_ready.notify_one();
            let _projection = task_dispatcher
                .begin_detached_projection("session-a", Uuid::new_v4())
                .await;
            task_projected.lock().await.push("detached-tool");
            task_completed.notify_one();
        });

        ready.notified().await;
        live.observe_frame(Some(42));
        projected.lock().await.push("frame-42");
        assert!(timeout(Duration::from_millis(25), completed.notified())
            .await
            .is_err());
        drop(live);
        timeout(Duration::from_secs(1), completed.notified())
            .await
            .expect("detached projection should run after the live page");
        task.await.unwrap();
        assert_eq!(
            *projected.lock().await,
            vec!["frame-41", "frame-42", "detached-tool"]
        );
    }

    #[tokio::test]
    async fn projection_sessions_do_not_block_each_other() {
        let dispatcher = AcpProjectionDispatcher::default();
        let _live = dispatcher
            .begin_live_page("session-a", Uuid::new_v4())
            .await;

        timeout(
            Duration::from_secs(1),
            dispatcher.begin_detached_projection("session-b", Uuid::new_v4()),
        )
        .await
        .expect("a different ACP session must remain dispatchable");
    }
}
