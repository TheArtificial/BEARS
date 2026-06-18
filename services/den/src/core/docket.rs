//! Compatibility shim — the Docket subsystem now lives in the `den-docket`
//! crate (see `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`). Re-exported here so
//! existing `crate::core::docket::…` paths keep compiling.

pub use den_docket::*;
