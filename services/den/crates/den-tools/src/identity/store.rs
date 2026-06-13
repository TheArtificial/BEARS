//! Capability seam for identity, membership, and policy lookups.
//!
//! [`BearDirectory`] backs the dispatcher's authorization (membership +
//! role resolution) and the `bear/*`, `user/*`, and `policy/*` read tools. The
//! `den-tools` executors own the JSON shaping; the `den` implementation owns the
//! `bears`/`user` DB access and returns the small DTOs below. See
//! `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B — dispatcher).

use async_trait::async_trait;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

/// Membership role string that grants Bear-admin privileges.
pub const BEAR_ROLE_ADMIN: &str = "admin";

/// Pure: does this membership role string grant Bear-admin privileges?
pub fn role_is_bear_admin(role: Option<&str>) -> bool {
    matches!(
        role.map(|s| s.trim().eq_ignore_ascii_case(BEAR_ROLE_ADMIN)),
        Some(true)
    )
}

/// Runtime-neutral projection of a Bear row for the `bear_get_self` tool.
#[derive(Debug, Clone)]
pub struct BearRecord {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub default_model: Option<String>,
    pub letta_agent_type: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Runtime-neutral projection of a Bear membership row.
#[derive(Debug, Clone)]
pub struct BearMemberRecord {
    pub user_id: i32,
    pub username: String,
    pub display_name: Option<String>,
    pub role: Option<String>,
}

/// Runtime-neutral projection of the authenticated user.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: i32,
    pub username: String,
    pub display_name: Option<String>,
    pub email_verified: bool,
    pub created_at: String,
}

#[async_trait]
pub trait BearDirectory: Send + Sync {
    /// Is `user_id` a member of `bear_id`?
    async fn user_may_use_bear(&self, user_id: i32, bear_id: Uuid) -> Result<bool, DenError>;

    /// Resolve the role registered for a `binding_id` (`bear_profile_bindings`).
    async fn registered_profile(
        &self,
        bear_id: Uuid,
        binding_id: &str,
    ) -> Result<Option<BearProfile>, DenError>;

    async fn bear_self(&self, bear_id: Uuid) -> Result<Option<BearRecord>, DenError>;

    async fn member_count(&self, bear_id: Uuid) -> Result<i64, DenError>;

    async fn members(&self, bear_id: Uuid) -> Result<Vec<BearMemberRecord>, DenError>;

    async fn current_user(&self, user_id: i32) -> Result<CurrentUser, DenError>;
}
