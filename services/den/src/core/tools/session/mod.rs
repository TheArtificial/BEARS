use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{
    config::Config,
    core::{acp_sessions, memory::MemoryStoreManager, tools::context::DenToolContext},
    errors::CustomError,
};

async fn patch_letta_conversation_summary(
    config: &Config,
    conversation_id: &str,
    summary: &str,
) -> Result<(), CustomError> {
    let base_url = config.letta_base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return Err(CustomError::System(
            "Letta is not configured (set LETTA_BASE_URL)".to_string(),
        ));
    }
    let url = format!("{base_url}/v1/conversations/{conversation_id}");
    let mut request = reqwest::Client::new()
        .patch(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&json!({ "summary": summary }));
    let key = config.letta_api_key.trim();
    if !key.is_empty() {
        request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = request
        .send()
        .await
        .map_err(|err| CustomError::System(format!("Letta patch conversation failed: {err}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(CustomError::System(format!(
            "Letta patch conversation HTTP {status}: {text}"
        )));
    }
    Ok(())
}

// The per-call context value now lives in `den-tools` (it is data, not a
// capability), so tool executors can move there. Re-exported here so existing
// `core::tools::session::DenToolInvocationContext` paths and the ~17 in-`den`
// construction sites keep resolving unchanged.
pub use den_tools::context::DenToolInvocationContext;

pub async fn invoke_den_tool(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    tool_name: &str,
    arguments: Value,
    context: DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let ctx = DenToolContext::new(pool, config, stores);
    den_tools::dispatch::invoke_den_tool(&ctx, tool_name, arguments, context)
        .await
        .map_err(CustomError::from)
}

pub(crate) struct DenConversationTitleOps<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) config: &'a Config,
}

#[async_trait::async_trait]
impl den_tools::conversation::ConversationTitleOps for DenConversationTitleOps<'_> {
    async fn patch_summary(
        &self,
        conversation_id: &str,
        summary: &str,
    ) -> Result<(), crate::errors::DenError> {
        patch_letta_conversation_summary(self.config, conversation_id, summary)
            .await
            .map_err(CustomError::into_den)
    }

    async fn set_title(
        &self,
        bear_id: uuid::Uuid,
        conversation_id: &str,
        title: &str,
    ) -> Result<u64, crate::errors::DenError> {
        acp_sessions::set_title_for_bear_conversation(self.pool, bear_id, conversation_id, title)
            .await
            .map_err(CustomError::into_den)
    }
}

#[cfg(test)]
mod test;
