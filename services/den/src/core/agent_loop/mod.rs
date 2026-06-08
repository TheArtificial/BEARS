//! Den-native in-process ReAct agent loop ([ADR-0035](../../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).

mod approvals;
mod context;
mod policy;
mod session_store;
mod step;
mod strategy;

pub use approvals::{
    create_native_approval, decide_native_approval, NativeApprovalDecision, NativeApprovalRow,
};
pub use context::assemble_agent_messages;
pub use session_store::{agent_loop_session_key, AgentLoopSession, AgentLoopSessionStore};
pub use step::run_agent_step_stream;
pub use policy::{select_strategy_profile, StrategyPolicyInput};
pub use strategy::StrategyProfile;
