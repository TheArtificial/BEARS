//! Native-used helpers and value types that grew up alongside the (now removed) self-hosted
//! Letta HTTP client. The legacy HTTP `LettaClient` is gone; this module retains the
//! Letta-named utilities still referenced by native code and tests: the runtime SSE stream
//! parser, tool-policy filtering, assistant display/title helpers, agent JSON projections, and
//! the model/tool option value types.

mod agent_diagnostics;
mod agent_document;
mod agent_prefill;
mod agent_summary;
mod assistant_display;
mod conversation_title;
mod model_options;
pub mod runtime_stream_parser;
pub mod tool_policy;

#[cfg(test)]
mod runtime_stream_parser_tests {
    include!("runtime_stream_parser_tests.rs");
}

/// How a pending-approval denial should be handled for a poisoned conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingApprovalDenialMode {
    PostToConversation,
    InspectOnly,
}

pub use agent_diagnostics::{LettaAgentDiagnostics, LettaBlockRow, LettaToolRow};
pub use agent_document::unwrap_letta_agent_document;
pub use agent_prefill::AgentBearPrefill;
pub use agent_summary::AgentSummary;
pub use assistant_display::{
    normalize_display_status_text, sanitize_visible_transcript_text, strip_letta_harness_for_user,
};
pub use conversation_title::{
    display_conversation_title, first_user_message_text_for_title, is_acceptable_derived_title,
    is_meaningful_conversation_title, UNTITLED_THREAD,
};
pub use model_options::{LettaModelOption, LettaToolOption};
pub use tool_policy::{filter_legacy_memory_tool_ids, is_legacy_memory_tool_name};
