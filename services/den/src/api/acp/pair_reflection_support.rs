use crate::{
    api::service::ApiState,
    core::{
        acp_sessions,
        bears::{db as bears_db, BearProfile},
        conversation_persistence,
        memory::{create_proposal, tools as sqlite_memory},
        memory_proposals::CreateMemoryProposal,
        memory_manager_head::{write_memfs_role_memory_entry, MemfsWriteRoleMemoryEntryRequest},
        pair_reflection::{self, CompletePairReflectionRun, CreatePairReflectionRun},
        reflection_conductor,
        runtime_conversations::{
            RuntimeConversationBackend, RuntimeConversationMessagesRequest,
            summarize_runtime_messages,
        },
    },
    errors::CustomError,
};

pub(crate) async fn run_pair_reflection_summary(
    state: &ApiState,
    session: &acp_sessions::AcpSessionRow,
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
    let messages_value = if state.config.uses_native_agent_runtime() {
        None
    } else if state.letta.is_enabled() {
        if let Some(conversation_id) = conversation_id {
            crate::core::acp_turn_runner_letta::LettaRuntimeCancellationBackend::new(
                state.letta.as_ref(),
            )
            .list_messages(RuntimeConversationMessagesRequest {
                conversation_id: conversation_id.to_string(),
                binding_id: None,
                limit: 20,
                before: None,
                ascending: false,
            })
            .await
            .ok()
        } else {
            None
        }
    } else {
        None
    };
    let canonical_summaries = if let Some(conversation_id) = conversation_id {
        if let Some(conversation) = conversation_persistence::get_conversation_for_external_id(
            &state.sqlx_pool,
            session.bear_id,
            conversation_id,
        )
        .await?
        {
            let rows = conversation_persistence::list_messages_page(
                &state.sqlx_pool,
                conversation.id,
                None,
                20,
            )
            .await?;
            rows.into_iter()
                .filter(|row| row.visibility == "visible")
                .map(|row| {
                    format!(
                        "{}: {}",
                        row.role.as_deref().unwrap_or(row.message_type.as_str()),
                        row.content_text.trim()
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let message_summaries = if canonical_summaries.is_empty() {
        summarize_runtime_messages(messages_value.as_ref())
    } else {
        canonical_summaries
    };
    let run = pair_reflection::create_run(
        &state.sqlx_pool,
        CreatePairReflectionRun {
            bear_id: session.bear_id,
            user_id: session.user_id,
            acp_session_id: &session.acp_session_id,
            conversation_id,
            trigger,
            considered_message_count: message_summaries.len() as i32,
            considered_memory_paths: Vec::new(),
            diagnostic: serde_json::json!({
                "phase": "pair_reflection_started",
                "conversation_id": conversation_id,
                "message_count": message_summaries.len(),
            }),
        },
    )
    .await?;
    let body = pair_reflection::render_pair_summary_markdown(
        &session.acp_session_id,
        conversation_id,
        trigger,
        &message_summaries,
    );
    let title = pair_reflection::summary_title_for_session(&session.acp_session_id);
    let (summary_path, summary_commit) = if state.config.uses_native_agent_runtime() {
        let artifact_id = format!("pair-reflection-{}", run.id);
        let logical_path = format!("pair/summaries/{artifact_id}.md");
        let written = sqlite_memory::sqlite_write_at_path(
            &state.memory_stores,
            state.config.as_ref(),
            session.bear_id,
            &logical_path,
            BearProfile::Pair.as_str(),
            &title,
            &body,
            serde_json::json!({
                "kind": "summary",
                "tags": ["pair-reflection", "session-summary"],
                "source": {
                    "human": { "user_id": session.user_id, "authenticated_by": "acp_token" },
                    "session": {
                        "acp_session_id": session.acp_session_id,
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
    } else {
        let request = MemfsWriteRoleMemoryEntryRequest {
            kind: "summary".to_string(),
            title: title.clone(),
            body,
            tags: vec!["pair-reflection".to_string(), "session-summary".to_string()],
            refs: None,
            lifecycle: Some(serde_json::json!({
                "scope": "role-local",
                "retention": "durable",
                "promotion": "maybe",
                "status": "active"
            })),
            source: Some(serde_json::json!({
                "human": { "user_id": session.user_id, "authenticated_by": "acp_token" },
                "session": {
                    "acp_session_id": session.acp_session_id,
                    "conversation_id": conversation_id,
                    "trigger": trigger
                },
                "reflection_run_id": run.id,
            })),
            author: None,
            conversation_id: conversation_id.map(str::to_string),
            session_id: Some(session.acp_session_id.clone()),
            acp_session_id: Some(session.acp_session_id.clone()),
            conversation_selection: Some(session.conversation_id.clone()),
            runtime_target: conversation_id.map(str::to_string),
            binding_id: None,
            profile: Some(pair_reflection::pair_reflection_role().as_str().to_string()),
            request_id: None,
        };
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| {
                CustomError::System(format!("MemFS pair reflection client build failed: {e}"))
            })?;
        let write_response = write_memfs_role_memory_entry(
            &http,
            &state.config.letta_memfs_service_url,
            session.bear_id,
            BearProfile::Pair.as_str(),
            &request,
        )
        .await?;
        let Some(write_response) = write_response else {
            pair_reflection::complete_run(
                &state.sqlx_pool,
                CompletePairReflectionRun {
                    id: run.id,
                    status: "skipped",
                    summary_path: None,
                    summary_commit: None,
                    diagnostic: serde_json::json!({"reason": "MemFS sidecar not configured"}),
                },
            )
            .await?;
            return Ok(());
        };
        (
            write_response.path,
            write_response.canonical_tip,
        )
    };
    let completed_run = pair_reflection::complete_run(
        &state.sqlx_pool,
        CompletePairReflectionRun {
            id: run.id,
            status: "completed",
            summary_path: Some(summary_path.as_str()),
            summary_commit: summary_commit.as_deref(),
            diagnostic: serde_json::json!({
                "phase": "pair_reflection_completed",
                "path": summary_path,
                "commit": summary_commit,
                "storage": if state.config.uses_native_agent_runtime() { "sqlite" } else { "memfs" },
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
            source_role: BearProfile::Pair,
            source_agent_id: pair_agent_id.clone(),
            source_paths: vec![summary_path.clone()],
            source_refs: serde_json::json!({
                "acp_session_id": session.acp_session_id,
                "conversation_id": conversation_id,
                "reflection_run_id": completed_run.id,
            }),
            suggested_action: "unspecified",
            target_ref: None,
            title: &format!("Review pair reflection summary: {}", session.acp_session_id),
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
            trigger: "pair_reflection",
            proposal_ids: vec![proposal.id],
        },
    )
    .await?;
    Ok(())
}

