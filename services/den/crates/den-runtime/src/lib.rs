//! `den-runtime` — Den's native agent runtime.
//!
//! This crate is the home of the native agent loop, provider/runtime, governance,
//! bears provisioning,
//! conversation storage, reflection, and the runtime-side memory/LLM glue
//! (DEN_CRATE_SPLIT_PLAN.md v1.4).
//!
//! Modules are migrated out of the `den` crate incrementally; this skeleton compiles on
//! its own and grows as each subsystem in the v1.4 extraction order lands here.

/// Plan-mode state machine and transitions (turn/plan coordination).
pub mod plan_mode;

/// Canonical BearWire gateway event model (`GatewayEvent`) + its SSE projection.
/// Protocol-neutral: edge adapters frame these events onto their own wire.
pub mod gateway_events;

/// Durable full-output artifacts for compacted tool results.
pub mod tool_output_artifacts;

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
/// conversation layer and the adapter edge).
pub mod conversation_ids;

/// Persisted BearWire event log for the Den ↔ armature wire.
pub mod bearwire_events;
/// Protocol-neutral coordinator decisions for turn obligations.
pub mod client_obligation_coordinator;
/// Final-request context budget estimation and component attribution.
pub mod context_budget;
/// Surface projection contracts for turn obligations.
pub mod surface_projection;
/// Typed identifiers for turn coordination.
pub mod turn_ids;
/// Persisted core turn obligations for tool/permission/human/channel waits.
pub mod turn_obligations;
/// Persisted core turn run lifecycle state machine.
pub mod turn_runs;
/// Persisted core model-step lifecycle.
pub mod turn_steps;

/// Runtime-side memory glue over the `den-memory` leaf: curation, curate-executor,
/// prompt-block store/blocks, proposals, observations, and admin inspection.
pub mod memory;

/// Runtime-side turn contracts (start/continue request inputs + materialization).
pub mod turn_runner;

/// The native agent loop: assembly, step streaming, approvals, transcript projection.
pub mod agent_loop;

/// The native runtime provider: profile turns, OpenAI streaming, web-chat loop.
pub mod native_runtime;

/// Reflection/curation worker subsystem: the memory-curate conductor + conversation lanes.
pub mod reflection;

/// Derived recall index client (ADR-0038): minimal Qdrant REST client for readiness +
/// collection bootstrap. Recall is optional; absence/unreachability degrades to keyword search.
pub mod recall;

// Flat aliases mirroring the den crate's former `core/mod.rs` runtime block, so the
// den-side re-export shims and intra-crate paths keep their familiar names.
pub use agent_assist::runtime_stream_parser;
pub use memory::bear_observations;
pub use memory::curate_executor as memory_curate_executor;
pub use reflection::conductor as reflection_conductor;
pub use reflection::conversations as reflection_conversations;
pub use runtime::bearwire_projection as runtime_bearwire_projection;
pub use runtime::compaction as runtime_compaction;
pub use runtime::compaction_observability as runtime_compaction_observability;
pub use runtime::compaction_store as runtime_compaction_store;
pub use runtime::conversations as runtime_conversations;
pub use runtime::pair_turn;
pub use runtime::provider as runtime_provider;
pub use runtime::role as role_runtime;
pub use runtime::role_registry as role_runtime_registry;
pub use runtime::turn_state;
