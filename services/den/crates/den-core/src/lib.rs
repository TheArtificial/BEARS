//! `den-core`: foundation leaf crate for the den workspace.
//!
//! Holds shared configuration, domain types, and errors with third-party
//! dependencies only. See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`.

pub mod config;
pub mod error;
pub mod metrics;
pub mod profile;

pub use error::DenError;
pub use profile::BearProfile;
