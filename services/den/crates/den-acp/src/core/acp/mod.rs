//! Agent Client Protocol (ACP) edge: sessions, tokens, the turn runner +
//! controller, and the ACP runtime glue. The shared turn/tool contracts
//! (events, tools, plan_mode, tool_turns) now live in the `den-runtime` crate.

pub mod runtime;
pub use den_runtime::acp_sessions as sessions;
pub use den_http::acp_tokens as tokens;
pub mod turn_runner;

#[cfg(test)]
mod runtime_tests {
    include!("runtime_tests.rs");
}

#[cfg(test)]
mod turn_runner_stream_tests {
    include!("turn_runner_stream_tests.rs");
}
