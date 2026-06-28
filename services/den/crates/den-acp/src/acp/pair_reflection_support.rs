use crate::service::DenState;
use den_http::errors::CustomError;
use den_memory::tools as sqlite_memory;
use den_service::{
    client_sessions,
    bears::{db as bears_db, BearProfile},
    memory_proposals::CreateMemoryProposal,
    pair_reflection::{CompletePairReflectionRun, CreatePairReflectionRun},
};
use den_runtime::{memory::create_proposal, reflection_conductor};

pub(crate) async fn run_pair_reflection_summary(
    state: &DenState,
    session: &client_sessions::ClientSessionRow,
    trigger: &str,
) -> Result<(), CustomError> {
    let conversation_id = session
        .resolved_conversation_id
        .as_deref()
        .or_else(|| {
            session
                .conversation_id
                .trim()
                .strip_prefix("")
                .filter(|_| false)
        })
        .or_else(|| {
            let raw = session.conversation_id.trim();
            if raw.starts_with("conv-")
                || crate::core::acp_runtime::is_native_runtime_conversation_id(raw)
            {
                Some(raw)
            } else {
                None
            }
        });
    let canonical_summaries = if let Some(conversation_id) = conversation_id {
        if let Some(conversation) = den_service::conversation::persistence::get_conversation_for_external_id(
            &state.sqlx_pool,
            session.bear_id,
            conversation_id,
        )
        .await?
        {
            let rows = den_service::conversation::persistence::list_messages_page(
                &state.sqlx_pool,
                conversation.id,
                None,
                20,
            )
            .await?;
            rows.into_iter()
                .filter_map(|row| row.to_model_transcript_message())
                .map(|message| format!("{}: {}", message.role, message.content.trim()))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let message_summaries = canonical_summaries;
    let run = den_service::pair_reflection::create_run(
        &state.sqlx_pool,
        CreatePairReflectionRun {
            bear_id: session.bear_id,
            user_id: session.user_id,
            acp_session_id: &session.client_session_id,
            conversation_id,
            trigger,
            considered_message_count: message_summaries.len() as i32,
            considered_memory_paths: Vec::new(),
            diagnostic: serde_json::json!({
                "phase": "den_service::pair_reflection_started",
                "conversation_id": conversation_id,
                "message_count": message_summaries.len(),
            }),
        },
    )
    .await?;
    let body = den_service::pair_reflection::render_pair_summary_markdown(
        &session.client_session_id,
        conversation_id,
        trigger,
        &message_summaries,
    );
    let title = den_service::pair_reflection::summary_title_for_session(&session.client_session_id);
    let (summary_path, summary_commit) = {
        let artifact_id = format!("pair-reflection-{}", run.id);
        let logical_path = format!("pair/summaries/{artifact_id}.md");
        let written = sqlite_memory::sqlite_write_at_path(
            &state.memory_stores,
            session.bear_id,
            &logical_path,
            BearProfile::Pair.as_str(),
            &title,
            &body,
            serde_json::json!({
                "kind": "summary",
                "tags": ["pair-reflection", "session-summary"],
                "source": {
                    "human": { "user_id": session.user_id, "authenticated_by": "armature_token" },
                    "session": {
                        "acp_session_id": session.client_session_id,
                        "conversation_id": conversation_id,
                        "trigger": trigger
                    },
                    "reflection_run_id": run.id,
                },
            }),
        )
        .await?;
        let path = written
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(&logical_path)
            .to_string();
        let memory_id = written
            .get("entry_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        (path, memory_id)
    };
    // Async-index the pair summary into derived recall (ADR-0038 Phase 1b); best-effort.
    den_runtime::reflection_conductor::enqueue_recall_index_if_enabled(
        &state.sqlx_pool,
        state.config.as_ref(),
        session.bear_id,
        "den_service::pair_reflection_summary",
    )
    .await;
    let completed_run = den_service::pair_reflection::complete_run(
        &state.sqlx_pool,
        CompletePairReflectionRun {
            id: run.id,
            status: "completed",
            summary_path: Some(summary_path.as_str()),
            summary_commit: summary_commit.as_deref(),
            diagnostic: serde_json::json!({
                "phase": "den_service::pair_reflection_completed",
                "path": summary_path,
                "commit": summary_commit,
                "storage": "sqlite",
            }),
        },
    )
    .await?;

    let pair_agent_id =
        bears_db::profile_binding_id(&state.sqlx_pool, session.bear_id, BearProfile::Pair)
            .await?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    let proposal = create_proposal(
        &state.sqlx_pool,
        state.config.as_ref(),
        &state.memory_stores,
        CreateMemoryProposal {
            bear_id: session.bear_id,
            source_profile: BearProfile::Pair,
            source_agent_id: pair_agent_id.clone(),
            source_paths: vec![summary_path.clone()],
            source_refs: serde_json::json!({
                "acp_session_id": session.client_session_id,
                "conversation_id": conversation_id,
                "reflection_run_id": completed_run.id,
            }),
            suggested_action: "unspecified",
            target_ref: None,
            title: &format!("Review pair reflection summary: {}", session.client_session_id),
            summary: "Pair reflection created a durable session summary; review for useful shared/work-visible knowledge.",
            rationale: "Pair reflection summaries may contain durable decisions, lessons, or work-visible knowledge that should be curated beyond pair-local memory.",
            proposed_content: None,
            proposed_patch: None,
            refs: serde_json::json!({
                "summary_path": summary_path,
                "summary_commit": summary_commit,
                "reflection_run_id": completed_run.id,
            }),
            sensitivity: "normal",
            requires_human: false,
            project_to_conversation: true,
        },
    )
    .await?;

    let reflection_date = time::OffsetDateTime::now_utc().date();
    let conversation_key = format!("memory_curate:{reflection_date}");
    reflection_conductor::enqueue_memory_curate_for_proposals(
        &state.sqlx_pool,
        reflection_conductor::ProposalEnqueueParams {
            bear_id: session.bear_id,
            binding_id: pair_agent_id.as_deref(),
            conversation_id,
            conversation_key: Some(&conversation_key),
            conversation_date: Some(reflection_date),
            trigger: "den_service::pair_reflection",
            proposal_ids: vec![proposal.id],
        },
    )
    .await?;
    Ok(())
}
