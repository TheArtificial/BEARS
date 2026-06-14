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
