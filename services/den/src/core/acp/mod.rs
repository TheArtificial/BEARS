//! Agent Client Protocol (ACP) runtime: sessions, tokens, the turn runner +
//! controller, tool turns, plan mode, the legacy Letta event mapping, and the
//! ACP runtime glue.

pub mod events;
pub mod plan_mode;
pub mod runtime;
pub mod sessions;
pub mod tokens;
pub mod tool_turns;
pub mod tools;
pub mod turn_controller;
pub mod turn_runner;

#[cfg(test)]
mod runtime_tests {
    include!("runtime_tests.rs");
}

#[cfg(test)]
mod turn_runner_stream_tests {
    include!("turn_runner_stream_tests.rs");
}
