//! `den-tools`: the descriptor/registry authority for Den's model-facing tools.
//!
//! Phase A of `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md`. This crate owns the *static*
//! tool surface and depends only on `den-core`:
//!
//! - [`constants`]: canonical and provider tool names.
//! - [`aliases`]: legacy provider aliases and built-in membership.
//! - [`arguments`]: argument shapes deserialized from tool calls.
//! - [`tool_descriptor_guidance`]: shared scope/side-effect/orientation language.
//! - [`display`]: the [`AcpToolDisplayDescriptor`] display shape.
//! - [`descriptor`]: the built-in Den tool descriptor table and profile gating.
//!
//! Tool executors (which need pool/config/stores) remain in the `den` crate until
//! the `ToolContext` seam (Phase B) inverts those capabilities behind traits.

pub mod aliases;
pub mod arguments;
pub mod constants;
pub mod context;
pub mod descriptor;
pub mod display;
pub mod memory;
pub mod preflight;
pub mod prompt_memory;
pub mod plan_mode;
pub mod review;
pub mod support;
pub mod work_surface;
pub mod tool_descriptor_guidance;
pub mod validation;
pub mod web;

pub use display::AcpToolDisplayDescriptor;
