//! Compatibility shims for the relocated environment payload builders.
//!
//! The renderers now live in `den_tools::environment::payloads` (pure). These
//! thin adapters preserve the original `den` signatures (`&user::User`, `&Config`)
//! for existing call sites/tests by mapping to the runtime-neutral inputs.

use serde_json::Value;

use den_tools::identity::CurrentUser;

use crate::{
    config::Config,
    core::{bears::BearProfile, tools::session::DenToolInvocationContext, user},
};

fn to_current_user(user: &user::User) -> CurrentUser {
    CurrentUser {
        id: user.id,
        username: user.username.clone(),
        display_name: Some(user.display_name.clone()),
        email_verified: user.email_verified.unwrap_or(false),
        created_at: String::new(),
    }
}

pub(crate) fn bear_environment_payload(
    context: &DenToolInvocationContext,
    config: &Config,
    role: BearProfile,
    current_user: Option<&user::User>,
    member_count: i64,
    memory_status: Value,
    adapter_runtime: Value,
) -> Value {
    let current_user = current_user.map(to_current_user);
    den_tools::environment::bear_environment_payload(
        context,
        !config.letta_memfs_service_url.trim().is_empty(),
        role,
        current_user.as_ref(),
        member_count,
        &memory_status,
        &adapter_runtime,
    )
}

pub(crate) fn session_info_payload(
    context: &DenToolInvocationContext,
    role: BearProfile,
    current_user: Option<&user::User>,
    member_count: i64,
    memory_status: Value,
) -> Value {
    let current_user = current_user.map(to_current_user);
    den_tools::environment::session_info_payload(
        context,
        role,
        current_user.as_ref(),
        member_count,
        &memory_status,
    )
}
