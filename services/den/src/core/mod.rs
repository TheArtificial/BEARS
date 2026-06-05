pub mod acp_letta_events;
pub mod acp_plan_mode;
pub mod acp_runtime;
pub mod acp_sessions;
pub mod acp_tokens;
pub mod acp_tool_turns;
pub mod acp_tools;
pub mod acp_turn_controller;
pub mod acp_turn_runner;
pub mod api_utils;
pub mod archived_conversations;
pub mod bears;
pub mod bifrost;
pub mod codepool;
pub mod conversation_events;
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
        pub use super::super::conversation_persistence::*;

        #[cfg(test)]
        mod non_acp_integration_tests {
            include!("conversation_persistence_non_acp_integration_tests.rs");
        }
    }
}
pub mod den_tools;
pub mod tools {
    pub mod memory {
        #[cfg(test)]
        mod apply_core_update_projection_tests {
            include!("den_tools_apply_core_update_projection_tests.rs");
        }
        #[cfg(test)]
        mod request_review_projection_tests {
            include!("den_tools_memory_request_review_projection_tests.rs");
        }
        #[cfg(test)]
        mod resolve_proposal_projection_tests {
            include!("den_tools_memory_resolve_proposal_projection_tests.rs");
        }
        #[cfg(test)]
        mod write_tests {
            include!("den_tools_memory_write_tests.rs");
        }
    }

    pub mod descriptor {
        #[cfg(test)]
        mod guidance_tests {
            include!("den_tools_descriptor_guidance_tests.rs");
        }
    }

    pub mod session {
        #[cfg(test)]
        mod info_tests {
            include!("den_tools_session_info_tests.rs");
        }
        #[cfg(test)]
        mod role_semantics_tests {
            include!("den_tools_session_role_semantics_tests.rs");
        }
    }

    pub mod work_surface {
        #[cfg(test)]
        mod orientation_tests {
            include!("den_tools_work_surface_orientation_tests.rs");
        }
        #[cfg(test)]
        mod scaffold_tests {
            include!("den_tools_work_surface_scaffold_tests.rs");
        }
    }

    pub mod workflow {
        #[cfg(test)]
        mod state_tests {
            include!("den_tools_workflow_state_tests.rs");
        }
    }
}
pub mod email;
pub mod letta;
pub mod letta_runtime_stream_parser;
pub mod memory_manager_head;
#[cfg(test)]
mod memory_manager_head_append_markdown_tests;
pub mod memory_proposals;
pub mod pair_reflection;
pub mod pair_turn;
pub mod prompt_memory_blocks;
pub mod reflection_conductor;
pub mod role_runtime;
#[cfg(test)]
mod role_runtime_tests;
pub mod runtime_compaction;
pub mod runtime_compaction_observability;
pub mod runtime_compaction_store;
pub mod runtime_contracts;
pub mod runtime_conversations;
pub mod runtime_provider;
#[cfg(test)]
mod runtime_compaction_eval_tests;
#[cfg(test)]
mod runtime_provider_tests;
pub mod s3;
pub mod tool_descriptor_guidance;
pub mod turn_state;
pub mod user;
pub mod web_policy;
pub mod work_plans;
