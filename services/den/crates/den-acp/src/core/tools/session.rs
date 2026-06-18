//! Re-export of the per-call tool invocation context.
//!
//! Builtin Den tool *execution* is injected via [`den_runtime::native_runtime::tool_invoker`] (a
//! [`den_runtime::native_runtime::RuntimeToolInvoker`]); the api edge does not own
//! the den-side tool composition. Only the invocation *context* data is shared here
//! so existing `core::tools::session::DenToolInvocationContext` paths keep resolving.
pub use den_core::tools::context::DenToolInvocationContext;
