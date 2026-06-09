//! Den-native in-process ReAct agent loop ([ADR-0035](../../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).

mod approvals;
mod assembler;
mod context;
mod policy;
mod runtime_context;
mod session_store;
mod session_stream;
mod step;
mod strategy;
mod tool_policy;
mod transcript;

pub use approvals::{
    create_native_approval, decide_native_approval, NativeApprovalDecision, NativeApprovalRow,
};
pub use assembler::{
    assemble_native_turn_messages, assemble_native_turn_messages_for_bear, AssembleTurnContext,
};
pub use context::assemble_agent_messages;
pub use session_store::{agent_loop_session_key, AgentLoopSession, AgentLoopSessionStore};
pub use session_stream::SessionTrackingStream;
pub use step::run_agent_step_stream;
pub use policy::{select_strategy_profile, StrategyPolicyInput};
pub use strategy::StrategyProfile;
pub use tool_policy::{maybe_pause_for_tool_approval, provider_tool_requires_approval, record_approval_decision};
