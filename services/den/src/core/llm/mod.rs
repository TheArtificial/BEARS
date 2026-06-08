//! OpenAI-compatible streaming inference client (Bifrost / `LLM_API_URL`).
//!
//! Emits [`crate::core::runtime_contracts::RuntimeStreamEvent`] directly — no Letta JSON intermediate.

mod client;
mod stream;

pub use client::{ChatCompletionRequest, ChatMessage, LlmClient, LlmToolDefinition};
pub use stream::{
    openai_sse_chunk_to_runtime_events, openai_sse_event_body_to_runtime_events,
    OpenAiStreamAccumulator, OpenAiStreamParseResult,
};

#[cfg(test)]
mod test;
