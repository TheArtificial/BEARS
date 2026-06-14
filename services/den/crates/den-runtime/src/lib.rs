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
