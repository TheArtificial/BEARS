//! Workflow tools are implemented in the `den` binary crate.
//!
//! Their argument and response shapes depend on `den-docket` domain types, while
//! `den-docket` depends on `den-core`. Keeping a coarse `WorkPlanOps` trait here
//! would preserve a stringly `serde_json::Value` seam. Instead, the binary-side
//! dispatcher wrapper intercepts workflow tools before calling the shared
//! `den_core::tools::dispatch` dispatcher.
