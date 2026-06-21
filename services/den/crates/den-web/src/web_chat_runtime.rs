use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use uuid::Uuid;

use crate::errors::CustomError;
use den_protocol::RuntimeStreamEvent;

pub type WebChatRuntimeStream =
    Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>>;

#[derive(Debug, Clone)]
pub struct WebChatRuntimeRequest {
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub chat_binding_id: String,
    pub user_id: i32,
    pub username: Option<String>,
    pub membership_role: Option<String>,
    pub conversation_id: String,
    pub session_id: String,
    pub prompt: String,
    pub request_id: Uuid,
}

pub trait WebChatRuntime: Send + Sync {
    fn stream_chat(
        &self,
        state: &crate::web::AppState,
        request: WebChatRuntimeRequest,
    ) -> futures::future::BoxFuture<'static, Result<WebChatRuntimeStream, CustomError>>;
}

#[derive(Debug, Default)]
pub struct NativeWebChatRuntime;

impl WebChatRuntime for NativeWebChatRuntime {
    fn stream_chat(
        &self,
        state: &crate::web::AppState,
        request: WebChatRuntimeRequest,
    ) -> futures::future::BoxFuture<'static, Result<WebChatRuntimeStream, CustomError>> {
        let pool = state.sqlx_pool().clone();
        let config = state.config.clone();
        Box::pin(async move {
            let stores = den_runtime::memory::MemoryStoreManager::new(config.as_ref());
            let deps = den_runtime::native_runtime::NativeRuntimeDeps {
                pool: &pool,
                config: config.as_ref(),
                stores: &stores,
            };
            let tool_invoker = den_runtime::native_runtime::tool_invoker().ok_or_else(|| {
                CustomError::System("builtin Den tool runtime is not initialized".to_string())
            })?;
            let runtime_stream = den_runtime::native_runtime::start_native_web_chat_turn_event_stream(
                den_runtime::native_runtime::NativeWebChatTurnParams {
                    deps: &deps,
                    bear_id: request.bear_id,
                    bear_slug: &request.bear_slug,
                    chat_binding_id: &request.chat_binding_id,
                    user_id: request.user_id,
                    username: request.username.as_deref(),
                    membership_role: request.membership_role.as_deref(),
                    conversation_id: &request.conversation_id,
                    session_id: &request.session_id,
                    prompt: &request.prompt,
                    request_id: request.request_id,
                    tool_invoker,
                },
            )
            .await?;

            let stream = futures::StreamExt::map(runtime_stream, |item| {
                item.map_err(crate::errors::CustomError::from)
            });
            Ok(Box::pin(stream) as WebChatRuntimeStream)
        })
    }
}

pub fn native_web_chat_runtime() -> Arc<dyn WebChatRuntime> {
    Arc::new(NativeWebChatRuntime)
}
