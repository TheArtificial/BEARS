//! Web tools — `den` boundary.
//!
//! The orchestration lives in `den_tools::web`; this module only provides the
//! concrete [`WebFetcher`](den_tools::web::WebFetcher) implementation
//! (`runtime`), wired into the dispatcher via `DenToolContext`. See
//! docs/roadmap/DEN_CRATE_SPLIT_PLAN.md (Phase B).

pub(crate) mod runtime;
