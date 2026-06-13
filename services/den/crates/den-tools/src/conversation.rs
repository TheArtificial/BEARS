//! Conversation metadata tool (`conversation_set_title`).
//!
//! Argument parsing, title normalization, and the "conversation not saved yet"
//! guards are pure and owned here; persistence flows through the
//! [`ConversationTitleOps`] seam. The Letta summary patch is legacy and drops out
//! once `core/letta` is removed (v0-legacy).

use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use den_core::DenError;

use crate::{arguments::SetConversationTitleArguments, context::DenToolInvocationContext, support::clean_optional};

#[async_trait]
pub trait ConversationTitleOps: Send + Sync {
    /// Patch the (legacy Letta) conversation summary.
    async fn patch_summary(&self, conversation_id: &str, summary: &str) -> Result<(), DenError>;

    /// Set the title on the Bear conversation; returns synced ACP-session count.
    async fn set_title(
        &self,
        bear_id: Uuid,
        conversation_id: &str,
        title: &str,
    ) -> Result<u64, DenError>;
}

pub async fn set_conversation_title(
    ops: &impl ConversationTitleOps,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, DenError> {
    let args: SetConversationTitleArguments = serde_json::from_value(arguments)?;
    let title = args.title.trim().chars().take(120).collect::<String>();
    if title.is_empty() {
        return Err(DenError::ValidationError(
            "conversation title cannot be empty".to_string(),
        ));
    }
    let conversation_id = clean_optional(&context.conversation_id).ok_or_else(|| {
        DenError::ValidationError(
            "current conversation is not saved yet; send a message before setting its title"
                .to_string(),
        )
    })?;
    if conversation_id == "default" || conversation_id.starts_with("new-") {
        return Err(DenError::ValidationError(
            "current conversation is not saved yet; send a message before setting its title"
                .to_string(),
        ));
    }
    ops.patch_summary(&conversation_id, &title).await?;
    let synced_acp_sessions = ops.set_title(context.bear_id, &conversation_id, &title).await?;
    Ok(json!({
        "ok": true,
        "conversation_id": conversation_id,
        "title": title,
        "synced_acp_sessions": synced_acp_sessions,
        "content": format!("Conversation title set to {title:?}."),
    }))
}
