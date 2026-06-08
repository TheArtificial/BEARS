//! Den-native in-process turn runtime ([ADR-0035](../../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).

mod openai_stream;
mod tools;
mod turn;

pub use openai_stream::openai_byte_stream_to_event_stream;
pub use tools::merge_den_and_client_tools;
pub use turn::{
    continue_native_acp_turn_event_stream, start_native_acp_turn_event_stream,
    NativeRuntimeConversationBackend,
};
