//! `den-runtime` — Den's native agent runtime.
//!
//! This crate is the implementation layer behind the `den-tools::ToolContext` seam and
//! the home of the native agent loop, provider/runtime, governance, bears provisioning,
//! conversation storage, reflection, and the runtime-side memory/LLM glue
//! (DEN_CRATE_SPLIT_PLAN.md v1.4).
//!
//! Modules are migrated out of the `den` crate incrementally; this skeleton compiles on
//! its own and grows as each subsystem in the v1.4 extraction order lands here.

/// ACP projection of the `den-tools` tool surface: tool classes, names, session policy,
/// and provider display/policy helpers. Shared by the runtime and the ACP edge.
pub mod acp_tools;

/// Plan-mode state machine and transitions (turn/plan coordination).
pub mod acp_plan_mode;

/// Canonical native runtime event model (`AcpGatewayEvent`) + its ACP-SSE adapter.
pub mod acp_events;

/// Active-turn coordination + tool-result requests (turn lifecycle).
pub mod acp_tool_turns;

/// Active-turn cancellation registry + turn controller (turn lifecycle).
pub mod acp_turn_controller;

/// The native agent runtime: provider/contracts, role runtime + registry, compaction,
/// conversations, turn state, pair-turn, and the BearWire projection.
pub mod runtime;

/// OpenAI-compatible streaming inference client (Bifrost / `LLM_API_URL`) + SSE→runtime
/// event mapping, over the `den-llm` leaf.
pub mod llm;

/// Agent-runtime support helpers: the runtime SSE stream parser, tool-policy filtering,
/// assistant display/title helpers, agent JSON projections, and model/tool option types.
pub mod agent_assist;

/// Pure conversation-id classification + normalization helpers (shared by the runtime
/// conversation layer and the ACP edge).
pub mod conversation_ids;

/// Bear provisioning, registry, managed blocks, runtime-plan, templates, and the `bears` DB.
pub mod bears;

/// Runtime-side memory glue over the `den-memory` leaf: curation, curate-executor,
/// prompt-block store/blocks, proposals, observations, and admin inspection.
pub mod memory;

/// Conversation event projection + persistence (events, message types, persistence, archive).
pub mod conversation;

/// ACP session store (conversation↔session mapping over Postgres).
pub mod acp_sessions;

/// Runtime-side ACP turn contracts (start/continue request inputs + materialization).
pub mod acp_turn_runner;

/// The native agent loop: assembly, step streaming, approvals, transcript projection.
pub mod agent_loop;

/// The native runtime provider: profile turns, OpenAI streaming, web-chat loop.
pub mod native_runtime;

// Flat aliases mirroring the den crate's former `core/mod.rs` runtime block, so the
// den-side re-export shims and intra-crate paths keep their familiar names.
pub use runtime::bearwire_projection as runtime_bearwire_projection;
pub use runtime::compaction as runtime_compaction;
pub use runtime::compaction_observability as runtime_compaction_observability;
pub use runtime::compaction_store as runtime_compaction_store;
pub use runtime::contracts as runtime_contracts;
pub use runtime::conversations as runtime_conversations;
pub use runtime::pair_turn;
pub use runtime::provider as runtime_provider;
pub use runtime::role as role_runtime;
pub use runtime::role_registry as role_runtime_registry;
pub use runtime::turn_state;
pub use llm::bifrost;
pub use agent_assist::runtime_stream_parser;
pub use memory::bear_observations;
pub use memory::curate_executor as memory_curate_executor;
pub use memory::proposals as memory_proposals;
pub use memory::prompt_block_store as prompt_memory_block_store;
pub use memory::prompt_blocks as prompt_memory_blocks;
pub use conversation::archived as archived_conversations;
pub use conversation::events as conversation_events;
pub use conversation::message_types as conversation_message_types;
pub use conversation::persistence as conversation_persistence;
