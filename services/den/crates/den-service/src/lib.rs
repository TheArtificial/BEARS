//! `den-service`: shared concrete service/application state for Den edges.
//!
//! Keep this below `den-runtime`: it owns process-local coordination and service
//! handles needed by HTTP edges, but not runtime execution or model turns.

pub mod bears;
pub mod bifrost;
pub mod conversation;
pub mod conversation_ids;
pub mod state;
pub mod tool_turns;
pub mod turn_controller;

pub use conversation::message_types as conversation_message_types;
pub use state::DenState;
