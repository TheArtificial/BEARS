//! `den-llm`: OpenAI-compatible streaming inference client (Bifrost / `LLM_API_URL`).
//!
//! Stable leaf crate (see `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`). Holds the LLM
//! client and request/response types; depends only on `den-core`. The SSE ->
//! `RuntimeStreamEvent` mapping lives in den-runtime to avoid a cycle.

mod client;
mod embeddings;
mod idle_byte_stream;

pub use client::{
    normalize_llm_model_handle, ChatCompletionRequest, ChatMessage, ChatToolCall,
    ChatToolCallFunction, LlmClient, LlmToolDefinition,
};
pub use embeddings::EmbeddingClient;
pub use idle_byte_stream::byte_stream_with_idle_timeout;
