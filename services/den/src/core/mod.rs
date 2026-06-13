pub mod acp_letta_events;
pub mod acp_plan_mode;
pub mod acp_runtime;
pub mod acp_runtime_test {
    #[cfg(test)]
    mod tests {
        include!("acp_runtime_tests.rs");
    }
}
pub mod acp_sessions;
pub mod acp_tokens;
pub mod acp_tool_turns;
pub mod acp_tools;
pub mod acp_turn_controller;
pub mod acp_turn_runner;
pub mod acp_turn_runner_test {
    #[cfg(test)]
    pub mod stream {
        include!("acp_turn_runner_stream_tests.rs");
    }
}
pub mod api_utils;
pub mod archived_conversations;
pub mod bears;
pub mod llm;
pub use llm::bifrost;
pub mod memory;
pub mod agent_loop;
pub mod native_runtime;
pub mod role_runtime_registry;
pub mod sandbox;
pub mod migration;
pub mod codepool;
pub mod conversation_events;
pub mod conversation_message_types;
pub mod conversation_persistence;
pub mod conversation {
    pub mod events {
        pub use super::super::conversation_events::*;

        #[cfg(test)]
        mod tests {
            include!("conversation_events_tests.rs");
        }
    }

    pub mod persistence {
        pub use super::super::conversation_message_types::*;
        pub use super::super::conversation_persistence::*;

        #[cfg(test)]
        mod non_acp_integration_tests {
            include!("conversation_persistence_non_acp_integration_tests.rs");
        }

        #[cfg(test)]
        mod idempotency_integration_tests {
            include!("conversation_idempotency_integration_tests.rs");
        }
    }
}
pub mod tools;
pub mod email;
pub mod letta;
pub use letta::runtime_stream_parser as letta_runtime_stream_parser;
pub mod memory_manager_head;
#[cfg(test)]
mod memory_manager_head_append_markdown_tests;
pub mod bear_observations;
pub mod memory_curate_executor;
pub mod memory_proposals;
pub mod reflection;
pub use reflection::conversations as reflection_conversations;
pub mod pair_reflection;
pub mod pair_turn;
pub mod prompt_memory_block_store;
pub mod prompt_memory_blocks;
pub use reflection::conductor as reflection_conductor;
pub mod role_runtime;
pub mod role_runtime_test {
    #[cfg(test)]
    mod tests {
        include!("role_runtime_tests.rs");
    }
}
pub mod runtime_test {}
pub mod runtime {
    pub mod bearwire_projection {
        pub use super::super::runtime_bearwire_projection::*;
    }

    pub mod compaction {
        pub use super::super::runtime_compaction::*;
    }

    pub mod compaction_observability {
        pub use super::super::runtime_compaction_observability::*;
    }

    pub mod compaction_store {
        pub use super::super::runtime_compaction_store::*;
    }

    pub mod contracts {
        pub use super::super::runtime_contracts::*;
    }

    pub mod conversations {
        pub use super::super::runtime_conversations::*;
    }

    pub mod provider {
        pub use super::super::runtime_provider::*;
    }
}
#[path = "runtime/compaction/mod.rs"]
pub mod runtime_compaction;
pub mod runtime_compaction_observability;
pub mod runtime_compaction_store;
#[path = "runtime/bearwire_projection/mod.rs"]
pub mod runtime_bearwire_projection;
#[path = "runtime/contracts/mod.rs"]
pub mod runtime_contracts;
pub mod runtime_conversations;
pub mod docket;
#[path = "runtime/provider/mod.rs"]
pub mod runtime_provider;
pub mod s3;
pub use tools::tool_descriptor_guidance;
pub mod turn_state;
pub mod user;
pub use tools::web_policy;
pub mod work_plans;
