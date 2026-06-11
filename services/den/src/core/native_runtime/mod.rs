//! Den-native in-process turn runtime ([ADR-0035](../../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).
//!
//! `start_native_profile_turn_event_stream` / `continue_native_profile_turn_event_stream` are the
//! capability-profile entry points for API-direct operating profiles (`pair`, and future
//! LLM-backed `curate` / `watch`). Rule-based curate (`memory_curate_executor`) and watch
//! (`observation_write`) stay on their non-loop paths until wired here.

mod openai_stream;
#[cfg(test)]
mod openai_stream_tests;
mod profile;
mod tools;
mod turn;

pub use openai_stream::openai_byte_stream_to_event_stream;
pub use profile::{is_native_api_direct_role, NativeCapabilityProfile};
pub use tools::merge_den_and_client_tools;
pub use turn::{
    continue_native_acp_turn_event_stream, continue_native_profile_turn_event_stream,
    start_native_acp_turn_event_stream, start_native_profile_turn_event_stream,
    NativeRuntimeConversationBackend,
};
