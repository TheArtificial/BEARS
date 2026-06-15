//! `den_core::tools`: the descriptor/registry authority for Den's model-facing tools.
//!
//! Owns the *static* tool surface plus the capability traits + dispatcher. It lives
//! in `den-core` (rather than a standalone `den-tools` crate) because it must sit
//! *below* `den-runtime`, which consumes the descriptor/work-surface/context types:
//!
//! - [`constants`]: canonical and provider tool names.
//! - [`aliases`]: legacy provider aliases and built-in membership.
//! - [`arguments`]: argument shapes deserialized from tool calls.
//! - [`tool_descriptor_guidance`]: shared scope/side-effect/orientation language.
//! - [`display`]: the [`ToolDisplayDescriptor`] display shape.
//! - [`descriptor`]: the built-in Den tool descriptor table and profile gating.
//!
//! The concrete executors (which need pool/config/stores and call `bears`/`memory`/
//! `reflection` in `den-runtime`) live *above* the runtime, in the `den` binary's
//! `core::tools`, wired in behind the capability traits / `RuntimeToolInvoker` seam.

pub mod aliases;
pub mod arguments;
pub mod constants;
pub mod context;
pub mod descriptor;
pub mod display;
pub mod memory;
pub mod preflight;
pub mod prompt_memory;
pub mod conversation;
pub mod dispatch;
pub mod environment;
pub mod identity;
pub mod plan_mode;
pub mod review;
pub mod support;
pub mod work_surface;
pub mod workflow;
pub mod tool_descriptor_guidance;
pub mod validation;
pub mod web;

pub use display::ToolDisplayDescriptor;
