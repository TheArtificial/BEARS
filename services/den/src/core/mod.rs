pub mod acp;
pub use den_runtime::acp_events;
pub use den_runtime::acp_plan_mode;
pub use acp::runtime as acp_runtime;
pub use acp::sessions as acp_sessions;
pub use acp::tokens as acp_tokens;
pub use den_runtime::acp_tool_turns;
pub use den_runtime::acp_tools;
pub use den_runtime::acp_turn_controller;
pub use acp::turn_runner as acp_turn_runner;
pub mod api_utils;
pub mod bears;
pub mod llm;
pub use llm::bifrost;
pub mod memory;
pub mod agent_loop;
pub mod native_runtime;
pub mod sandbox;
pub mod migration;
pub mod conversation;
pub use conversation::archived as archived_conversations;
pub use conversation::events as conversation_events;
pub use conversation::message_types as conversation_message_types;
pub use conversation::persistence as conversation_persistence;
pub mod tools;
pub mod email;
pub mod agent_assist;
pub use agent_assist::runtime_stream_parser;
pub use memory::bear_observations;
pub use memory::curate_executor as memory_curate_executor;
pub use memory::proposals as memory_proposals;
pub mod reflection;
pub use reflection::conversations as reflection_conversations;
pub mod pair_reflection;
pub use memory::prompt_block_store as prompt_memory_block_store;
pub use memory::prompt_blocks as prompt_memory_blocks;
pub use reflection::conductor as reflection_conductor;
pub use den_runtime::runtime;
pub use den_runtime::{
    pair_turn, role_runtime, role_runtime_registry, runtime_bearwire_projection,
    runtime_compaction, runtime_compaction_observability, runtime_compaction_store,
    runtime_contracts, runtime_conversations, runtime_provider,
};
pub mod docket;
pub mod s3;
pub use tools::tool_descriptor_guidance;
pub use den_runtime::turn_state;
pub mod user;
pub use tools::web_policy;
pub mod work_plans;

// Cross-layer bridge tests: they exercise den_runtime modules together with
// den-only modules (native_runtime / acp_turn_controller), so they live here in the
// `den` crate rather than in den-runtime. Relocated during the v1.4 runtime lift.
#[cfg(test)]
mod runtime_role_bridge_tests;
#[cfg(test)]
mod runtime_bearwire_bridge_tests;
