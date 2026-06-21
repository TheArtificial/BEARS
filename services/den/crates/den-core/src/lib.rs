//! `den-core`: foundation leaf crate for the den workspace.
//!
//! Holds shared configuration, domain types, and errors with third-party
//! dependencies only. See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

pub mod client_tools;
pub mod config;
pub mod error;
pub mod governance;
pub mod ids;
pub mod metrics;
pub mod profile;

/// Model-facing tool surface: canonical/provider names, argument shapes, the
/// descriptor table + profile gating, capability traits, and the dispatcher.
/// (The concrete DB-backed executors live in the `den` binary's `core::tools`.)
pub mod tools;

pub use error::DenError;
pub use governance::{GovernanceMode, RunMode};
pub use ids::{BearId, ConversationId, SessionId, UserId};
pub use profile::{BearProfile, BearStance};
