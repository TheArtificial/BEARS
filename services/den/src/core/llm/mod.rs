//! OpenAI-compatible streaming inference client (Bifrost / `LLM_API_URL`).
//!
//! Emits [`crate::core::runtime_contracts::RuntimeStreamEvent`] directly — no Letta JSON intermediate.

mod client;
mod idle_byte_stream;
mod stream;

pub(crate) use idle_byte_stream::byte_stream_with_idle_timeout;
pub use client::{
    ChatCompletionRequest, ChatMessage, ChatToolCall, ChatToolCallFunction, LlmClient,
    LlmToolDefinition,
};
pub use stream::{
    openai_sse_chunk_to_runtime_events, openai_sse_event_body_to_runtime_events,
    openai_sse_frame_to_runtime_events, OpenAiStreamAccumulator, OpenAiStreamParseResult,
};

#[cfg(test)]
mod test;
