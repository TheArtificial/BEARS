//! OpenAI-compatible streaming inference client (Bifrost / `LLM_API_URL`).
//!
//! Emits [`crate::runtime_contracts::RuntimeStreamEvent`] directly — no Letta JSON intermediate.

pub mod bifrost;
pub mod model_registry;
mod stream;

pub use den_llm::byte_stream_with_idle_timeout;
pub use den_llm::{
    bifrost_key_selection_error, execution_fallback_model_handles,
    preferred_api_style_for_model, ChatCompletionRequest, ChatMessage, ChatToolCall,
    ChatToolCallFunction, EmbeddingClient, LlmApiStyle, LlmClient, LlmRequestTelemetry,
    LlmToolDefinition,
};
pub use stream::{
    openai_sse_chunk_to_runtime_events, openai_sse_event_body_to_runtime_events,
    openai_sse_frame_to_runtime_events, openai_sse_frame_to_runtime_events_with_diagnostics,
    responses_sse_frame_to_runtime_events, OpenAiStreamAccumulator, OpenAiStreamDiagnostics,
    OpenAiStreamParseResult, ResponsesStreamAccumulator,
};

#[cfg(test)]
mod test;
