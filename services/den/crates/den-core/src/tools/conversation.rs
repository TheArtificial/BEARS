//! Conversation metadata tool (`conversation_set_title`).
//!
//! Argument parsing, title normalization, and the "conversation not saved yet"
//! guards are pure and owned here; persistence flows through the
//! [`ConversationTitleOps`] seam (native Bear-conversation title store).

use serde_json::{json, Value};
use uuid::Uuid;

use crate::DenError;

use crate::tools::{
    arguments::SetConversationTitleArguments, context::DenToolInvocationContext,
    support::clean_optional,
};

// Native async fn in trait: workspace-internal, consumed via generic bounds /
// concrete impls only (never `dyn`), so Send flows through monomorphization.
#[allow(async_fn_in_trait)]
pub trait ConversationTitleOps: Send + Sync {
    /// Set the title on the Bear conversation; returns synced client-session count.
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
    let synced_acp_sessions = ops
        .set_title(context.bear_id, &conversation_id, &title)
        .await?;
    Ok(json!({
        "ok": true,
        "conversation_id": conversation_id,
        "title": title,
        "synced_acp_sessions": synced_acp_sessions,
        "content": format!("Conversation title set to {title:?}."),
    }))
}
