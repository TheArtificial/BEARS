//! `den`-side wiring for `memory_write_entry`.
//!
//! Orchestration (role gating, validation, source merging, entry construction)
//! lives in `den-tools`; here we resolve the authoring user (a capability),
//! delegate to the den-tools executor over the shared [`DenRoleMemoryStore`], and
//! map `DenError` back to `CustomError`. The compat wrappers/re-exports preserve
//! the paths used across `den` (`source_acp_session_id`) and tests.

use serde_json::Value;
use sqlx::PgPool;

use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        tools::{memory_read::DenRoleMemoryStore, session::DenToolInvocationContext},
        user,
    },
    errors::CustomError,
};

pub(crate) use den_tools::memory::source_acp_session_id;
#[cfg(test)]
pub(crate) use den_tools::memory::MemoryWriteEntryArguments;

/// Compat wrapper: resolves the human identity from a `user::User` and delegates
/// to the relocated primitive-based merge in `den-tools`.
pub(crate) fn merge_memory_entry_source_with_human(
    source: Option<Value>,
    context: &DenToolInvocationContext,
    current_user: Option<&user::User>,
) -> Option<Value> {
    den_tools::memory::merge_memory_entry_source_with_human(
        source,
        context,
        current_user.map(|user| user.username.clone()),
        current_user.map(|user| user.display_name.clone()),
    )
}

pub(crate) async fn write_memory_entry(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let current_user = user::user_by_id(pool, context.user_id).await.ok();
    let author_username = current_user.as_ref().map(|user| user.username.clone());
    let author_display_name = current_user.as_ref().map(|user| user.display_name.clone());
    let store = DenRoleMemoryStore::new(config);
    den_tools::memory::write_memory_entry(
        &store,
        context,
        role,
        arguments,
        author_username,
        author_display_name,
    )
    .await
    .map_err(CustomError::from)
}
