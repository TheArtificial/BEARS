use serde_json::Value;
use sqlx::PgPool;

use crate::{
    config::Config,
    errors::CustomError,
    core::tools::context::DenToolContext,
};
use den_runtime::{
    acp_sessions,
    memory::MemoryStoreManager,
};

// The per-call context value now lives in `den-tools` (it is data, not a
// capability), so tool executors can move there. Re-exported here so existing
// `core::tools::session::DenToolInvocationContext` paths and the ~17 in-`den`
// construction sites keep resolving unchanged.
pub use den_core::tools::context::DenToolInvocationContext;

pub async fn invoke_den_tool(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    tool_name: &str,
    arguments: Value,
    context: DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let ctx = DenToolContext::new(pool, config, stores);
    den_core::tools::dispatch::invoke_den_tool(&ctx, tool_name, arguments, context)
        .await
        .map_err(CustomError::from)
}

pub(crate) struct DenConversationTitleOps<'a> {
    pub(crate) pool: &'a PgPool,
}

#[async_trait::async_trait]
impl den_core::tools::conversation::ConversationTitleOps for DenConversationTitleOps<'_> {
    async fn set_title(
        &self,
        bear_id: uuid::Uuid,
        conversation_id: &str,
        title: &str,
    ) -> Result<u64, crate::errors::DenError> {
        acp_sessions::set_title_for_bear_conversation(self.pool, bear_id, conversation_id, title)
            .await
    }
}

#[cfg(test)]
mod test;
