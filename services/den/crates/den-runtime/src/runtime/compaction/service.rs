//! Back-compat re-exports — implementation lives in [`lifecycle`](super::lifecycle).

pub use super::lifecycle::{
    enqueue_compaction_after_turn, load_compaction_context, on_turn_assemble_compaction,
    prepare_turn_compaction, render_compaction_prompt_context, run_compaction_job,
    TurnCompactionState, TurnCompactionTrigger,
};
