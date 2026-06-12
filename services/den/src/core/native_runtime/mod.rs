//! Den-native in-process turn runtime ([ADR-0035](../../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).
//!
//! `start_native_profile_turn_event_stream` / `continue_native_profile_turn_event_stream` are the
//! capability-profile entry points for API-direct operating profiles (`pair`, `curate`; `watch`
//! stays rule-based). Browser web chat uses `start_native_web_chat_turn_event_stream` for
//! `BearProfile::Chat` when `AGENT_RUNTIME=native`. Rule-based curate (`memory_curate_executor`)
//! runs first; when briefing items remain under native runtime,
//! `run_native_profile_turn_collect_assistant_text` can add an LLM briefing turn projected into the
//! memory_curate conversation.

mod openai_stream;
#[cfg(test)]
mod openai_stream_tests;
mod profile;
mod profile_briefing;
mod tools;
mod turn;
mod web_chat_loop;

pub use openai_stream::openai_byte_stream_to_event_stream;
pub use profile::{is_native_api_direct_role, NativeCapabilityProfile};
pub use profile_briefing::compose_curate_briefing_prompt;
pub use tools::{
    chat_turn_is_capabilities_meta_query, merge_den_and_client_tools,
};
pub use turn::{
    continue_native_acp_turn_event_stream, continue_native_profile_turn_event_stream,
    run_native_profile_turn_collect_assistant_text, start_native_acp_turn_event_stream,
    start_native_profile_turn_event_stream, start_native_web_chat_turn_event_stream,
    NativeRuntimeConversationBackend, NativeRuntimeDeps, NativeWebChatTurnParams,
};
