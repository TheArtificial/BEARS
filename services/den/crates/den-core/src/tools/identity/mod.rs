//! Identity / membership / policy tool executors + dispatcher authorization.
//!
//! These executors own the JSON shaping for the `bear/*`, `user/*`, `policy/*`,
//! `capabilities/*`, and `channel/*` read tools, plus the dispatcher's
//! authorization (`authorize_context`/`context_role`/`authorize_tool_for_profile`).
//! All DB access flows through the [`BearDirectory`] seam.

mod store;

pub use store::{
    role_is_bear_admin, BearDirectory, BearMemberRecord, BearRecord, CurrentUser, BEAR_ROLE_ADMIN,
};

use serde_json::{json, Value};

use den_core::{BearProfile, DenError};

use crate::{
    context::DenToolInvocationContext,
    descriptor::{builtin_den_tool_descriptors, builtin_den_tool_descriptors_for_profile},
};

/// Pure: the trusted channel-context echo for `channel_get_context`.
pub fn channel_context(context: &DenToolInvocationContext) -> Value {
    json!({
        "bear_id": context.bear_id,
        "binding_id": context.binding_id,
        "profile": context.profile,
        "user_id": context.user_id,
        "conversation_id": context.conversation_id,
        "session_id": context.session_id,
        "request_id": context.request_id,
        "channel": context.channel,
    })
}

/// Pure: the callable-tool descriptors for the caller's resolved role.
pub fn list_capabilities_self(context: &DenToolInvocationContext, role: BearProfile) -> Value {
    let descriptors = builtin_den_tool_descriptors_for_profile(role);
    json!({
        "bear_id": context.bear_id,
        "channel": context.channel,
        "capabilities": descriptors,
    })
}

pub async fn get_bear_self(
    dir: &impl BearDirectory,
    context: &DenToolInvocationContext,
) -> Result<Value, DenError> {
    let bear = dir
        .bear_self(context.bear_id)
        .await?
        .ok_or_else(|| DenError::NotFound("bear not found".to_string()))?;
    let member_count = dir.member_count(context.bear_id).await?;
    Ok(json!({
        "bear": {
            "bear_id": bear.id,
            "slug": bear.slug,
            "name": bear.name,
            "description": bear.description,
            "default_model": bear.default_model,
            "letta_agent_type": bear.letta_agent_type,
            "member_count": member_count,
            "created_at": bear.created_at,
            "updated_at": bear.updated_at
        },
        "viewer": {
            "user_id": context.user_id,
            "username": context.username,
            "membership_role": context.membership_role,
            "is_bear_admin": role_is_bear_admin(context.membership_role.as_deref())
        }
    }))
}

pub async fn get_current_user(
    dir: &impl BearDirectory,
    context: &DenToolInvocationContext,
) -> Result<Value, DenError> {
    let current = dir.current_user(context.user_id).await?;
    Ok(json!({
        "user": {
            "user_id": current.id,
            "username": current.username,
            "display_name": current.display_name,
            "email_verified": current.email_verified,
            "created_at": current.created_at
        },
        "bear_membership": {
            "bear_id": context.bear_id,
            "role": context.membership_role,
            "is_bear_admin": role_is_bear_admin(context.membership_role.as_deref())
        }
    }))
}

pub async fn list_bear_members(
    dir: &impl BearDirectory,
    context: &DenToolInvocationContext,
) -> Result<Value, DenError> {
    let members = dir.members(context.bear_id).await?;
    let can_manage_members = role_is_bear_admin(context.membership_role.as_deref());
    let member_values: Vec<Value> = members
        .into_iter()
        .map(|member| {
            json!({
                "user_id": member.user_id,
                "username": member.username,
                "display_name": member.display_name,
                "role": member.role,
            })
        })
        .collect();
    Ok(json!({
        "bear_id": context.bear_id,
        "members": member_values,
        "policy": {
            "viewer_role": context.membership_role,
            "can_manage_members": can_manage_members,
            "redacted_fields": ["email"]
        }
    }))
}

pub async fn policy_self(
    dir: &impl BearDirectory,
    context: &DenToolInvocationContext,
) -> Result<Value, DenError> {
    let member_count = dir.member_count(context.bear_id).await?;
    let is_bear_admin = role_is_bear_admin(context.membership_role.as_deref());
    Ok(json!({
        "bear_id": context.bear_id,
        "user_id": context.user_id,
        "membership_role": context.membership_role,
        "is_bear_admin": is_bear_admin,
        "can_chat": true,
        "can_read_bear_profile": true,
        "can_list_members": true,
        "can_manage_members": is_bear_admin,
        "can_list_capabilities": true,
        "can_read_channel_context": true,
        "member_count": member_count,
        "policy_notes": [
            "Den tool calls are scoped to the current trusted bear/user context.",
            "Emails and authentication internals are not exposed through these tools."
        ]
    }))
}

/// Resolve and verify the caller's role from its `binding_id`.
pub async fn context_role(
    dir: &impl BearDirectory,
    context: &DenToolInvocationContext,
) -> Result<BearProfile, DenError> {
    let agent_id = context.binding_id.trim();
    if agent_id.is_empty() {
        return Err(DenError::Authorization(
            "Den tool context is missing binding_id".to_string(),
        ));
    }
    let registered_profile = dir
        .registered_profile(context.bear_id, agent_id)
        .await?
        .ok_or_else(|| {
            DenError::Authorization("binding_id is not registered for this bear".to_string())
        })?;
    if let Some(declared_profile) = context.profile {
        if declared_profile != registered_profile {
            return Err(DenError::Authorization(format!(
                "Den tool context profile `{declared_profile}` does not match registered profile `{registered_profile}` for binding_id"
            )));
        }
    }
    Ok(registered_profile)
}

/// Verify membership, then resolve the caller's role.
pub async fn authorize_context(
    dir: &impl BearDirectory,
    context: &DenToolInvocationContext,
) -> Result<BearProfile, DenError> {
    if !dir.user_may_use_bear(context.user_id, context.bear_id).await? {
        return Err(DenError::Authorization(
            "user is not a member of this bear".to_string(),
        ));
    }
    context_role(dir, context).await
}

/// Reject tools the resolved role is not permitted to call.
pub fn authorize_tool_for_profile(tool_name: &str, role: BearProfile) -> Result<(), DenError> {
    let descriptor = builtin_den_tool_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.name == tool_name)
        .ok_or_else(|| DenError::NotFound(format!("unknown Den tool: {tool_name}")))?;
    if descriptor.allows_profile(role) {
        Ok(())
    } else {
        Err(DenError::Authorization(format!(
            "Den tool `{tool_name}` is not available to the `{role}` role"
        )))
    }
}
