//! `DenToolInvocationContext`: the per-call value passed to every tool executor.
//!
//! This is **data, not a capability** (per the Phase B design rules): plain,
//! serializable per-invocation identity/session/channel info. It lives in
//! `den-tools` so executors can move here; the `den` crate re-exports it at
//! `core::tools::session::DenToolInvocationContext` for existing call sites and
//! is responsible for constructing it. See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

use crate::BearProfile;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::tools::arguments::DenToolChannelContext;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DenToolInvocationContext {
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub binding_id: String,
    pub profile: Option<BearProfile>,
    pub user_id: i32,
    pub username: Option<String>,
    pub membership_role: Option<String>,
    pub conversation_id: String,
    pub session_id: String,
    #[serde(default)]
    pub acp_session_id: Option<String>,
    #[serde(default)]
    pub conversation_selection: Option<String>,
    #[serde(default)]
    pub runtime_target: Option<String>,
    #[serde(default)]
    pub workspace_roots: Vec<String>,
    #[serde(default)]
    pub session_policy: Option<Value>,
    #[serde(default)]
    pub activity: Option<Value>,
    #[serde(default)]
    pub runtime: Option<Value>,
    #[serde(default)]
    pub context_budget: Option<Value>,
    pub request_id: Option<String>,
    #[serde(default)]
    pub channel: DenToolChannelContext,
}
