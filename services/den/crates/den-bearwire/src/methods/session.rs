use axum::http::HeaderMap;
use den_core::DenError;
use den_docket::{
    DocketExecutionAttemptAuthorize, DocketExecutionAttemptOwner, DocketExecutionAttemptStart,
    DocketService, PgDocketService,
};
use serde_json::{json, Value};
use sqlx::PgPool;

use bearwire_protocol::{
    methods::{
        RunStartRequest, SessionCurrentTaskClearRequest, SessionCurrentTaskSelectionRequest,
        SessionCurrentTaskStartRequest, SessionIdRequest, SessionModelSetRequest,
        SessionOpenRequest, SessionStateRequest,
    },
    wire::BearWireEvent,
};
use den_http::errors::CustomError;
use den_runtime::{
    agent_loop::{LedgerEvidenceRef, LoopControlDecisionKind, LoopControlLedgerInput},
    bearwire_events,
    conversation_review::{
        ConversationReview, ConversationReviewFinding, ConversationReviewFindingDetail,
        ConversationReviewTrigger, FindingSource,
    },
    current_task::{preview_pair_current_task_selection, select_pair_current_task},
    pair_reflection::create_pair_reflection_proposals_from_latest_summary,
    runtime::compaction::{prepare_turn_compaction, TurnCompactionState, TurnCompactionTrigger},
    runtime::task_context::{
        active_docket_execution_lookup_for_session, resolve_runtime_task_context,
        RuntimeTaskResolveRequest,
    },
    turn_obligations,
};
use den_service::{
    bears::{db as bears_db, BearProfile},
    client_sessions, DenState,
};

use crate::auth::{authenticate_for_bear_slug, authenticated_bear};
use crate::methods::{parse_params, DEFAULT_CLIENT};

pub async fn reflect_open_sessions_once(state: &DenState) -> Result<usize, CustomError> {
    let candidates = client_sessions::list_open_reflection_candidates(
        &state.sqlx_pool,
        client_sessions::OpenReflectionCandidatesParams {
            stale_after_minutes: 30,
            activity_threshold: 20,
            limit: 25,
        },
    )
    .await?;
    let mut processed = 0;
    for candidate in candidates {
        let session = candidate.session();
        match reflect_pair_session(
            &state.sqlx_pool,
            state,
            &session,
            &candidate.reflection_trigger,
        )
        .await
        {
            Ok(reflection_payload) => {
                processed += 1;
                let mut event = BearWireEvent::ephemeral(
                    "session.reflected",
                    json!({
                        "session_id": session.client_session_id,
                        "bear_slug": session.bear_slug,
                        "trigger": candidate.reflection_trigger,
                        "event_count": candidate.event_count,
                        "latest_compaction_source_end_seq": candidate.latest_compaction_source_end_seq,
                        "last_reflected_source_end_seq": candidate.last_reflected_source_end_seq,
                        "pair_reflection": reflection_payload,
                    }),
                );
                event.bear_id = Some(session.bear_id.to_string());
                event.human_id = Some(session.user_id.to_string());
                event.session_id = Some(session.client_session_id.clone());
                if let Err(error) = bearwire_events::append_bearwire_event(
                    &state.sqlx_pool,
                    &session.client_session_id,
                    Some(session.bear_id),
                    Some(session.user_id),
                    event,
                )
                .await
                {
                    tracing::warn!(session_id = %session.client_session_id, error = %error, "failed to record open-session reflection event");
                }
            }
            Err(error) => {
                tracing::warn!(session_id = %session.client_session_id, error = %error, "open-session pair reflection failed");
            }
        }
    }
    Ok(processed)
}

async fn reflect_pair_session(
    pool: &PgPool,
    state: &DenState,
    session: &client_sessions::ClientSessionRow,
    trigger: &str,
) -> Result<Value, CustomError> {
    let conversation_id = session
        .resolved_conversation_id
        .as_deref()
        .unwrap_or(&session.conversation_id)
        .to_string();
    let compaction_state = prepare_turn_compaction(
        pool,
        &state.config,
        session.bear_id,
        &conversation_id,
        BearProfile::Pair,
        TurnCompactionTrigger::ConversationReview,
    )
    .await?;
    let output = create_pair_reflection_proposals_from_latest_summary(
        pool,
        &state.config,
        &state.memory_stores,
        session.bear_id,
        &conversation_id,
        &session.client_session_id,
    )
    .await
    .map_err(CustomError::from)?;
    let review = build_pair_conversation_review(
        conversation_id.clone(),
        session.client_session_id.clone(),
        trigger,
        compaction_state.as_ref(),
        output.candidate_count,
        output.source_message_start_seq,
        output.source_message_end_seq,
    );
    Ok(json!({
        "status": if output.skipped_reason.is_some() { "skipped" } else { "processed" },
        "trigger": trigger,
        "conversation_review": review,
        "skipped_reason": output.skipped_reason,
        "candidate_count": output.candidate_count,
        "discarded_count": output.discarded_count,
        "discarded_reasons": output.discarded_reasons,
        "dropped_followup_count": output.dropped_followup_count,
        "proposal_ids": output.created_proposal_ids,
        "source_message_start_seq": output.source_message_start_seq,
        "source_message_end_seq": output.source_message_end_seq,
    }))
}

fn build_pair_conversation_review(
    conversation_id: String,
    client_session_id: String,
    trigger: &str,
    compaction_state: Option<&TurnCompactionState>,
    memory_candidate_count: usize,
    source_message_start_seq: Option<i64>,
    source_message_end_seq: Option<i64>,
) -> ConversationReview {
    let refs = source_seq_refs(source_message_start_seq, source_message_end_seq);
    let mut findings = Vec::new();

    if let Some(state) = compaction_state {
        if state.decision.is_some() {
            findings.push(ConversationReviewFinding {
                source: FindingSource::runtime(refs.clone()),
                detail: ConversationReviewFindingDetail::CompactionNeeded {
                    reason: "Conversation review produced a compaction artifact.".to_string(),
                },
            });
        }
    }

    if memory_candidate_count > 0 {
        findings.push(ConversationReviewFinding {
            source: FindingSource::runtime(refs),
            detail: ConversationReviewFindingDetail::MemoryReflectionCandidate {
                reason: format!(
                    "Pair reflection found {memory_candidate_count} memory candidate(s)."
                ),
            },
        });
    }

    ConversationReview::new(
        conversation_id,
        Some(client_session_id),
        None,
        conversation_review_trigger_from_reflection_trigger(trigger),
        findings,
    )
}

fn conversation_review_trigger_from_reflection_trigger(trigger: &str) -> ConversationReviewTrigger {
    match trigger {
        "session_close" => ConversationReviewTrigger::SessionClose,
        "manual" => ConversationReviewTrigger::Manual,
        _ => ConversationReviewTrigger::OpenSessionSweep,
    }
}

fn source_seq_refs(start_seq: Option<i64>, end_seq: Option<i64>) -> Vec<String> {
    match (start_seq, end_seq) {
        (Some(start), Some(end)) => vec![format!("conversation_seq:{start}-{end}")],
        (Some(start), None) => vec![format!("conversation_seq:{start}-")],
        (None, Some(end)) => vec![format!("conversation_seq:-{end}")],
        (None, None) => Vec::new(),
    }
}

fn resolved_or_stored_conversation_id(session: &client_sessions::ClientSessionRow) -> &str {
    session
        .resolved_conversation_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(session.conversation_id.as_str())
}

async fn session_state_payload(
    state: &DenState,
    session: client_sessions::ClientSessionRow,
    work_enabled: bool,
) -> Result<Value, CustomError> {
    let conversation_external_id = resolved_or_stored_conversation_id(&session);
    let conversation_runtime_id = conversation_external_id.to_string();
    let latest_context_budget =
        den_service::conversation::persistence::get_conversation_for_external_id(
            &state.sqlx_pool,
            session.bear_id,
            conversation_external_id,
        )
        .await?
        .and_then(|conversation| conversation.latest_context_budget);
    let trusted_workspace = session.trusted_workspace_context();
    let runtime_session_live = den_runtime::native_runtime::native_client_session_exists(
        &conversation_runtime_id,
        &session.client_session_id,
    );
    let runtime_task_context = if work_enabled {
        let context = resolve_runtime_task_context(
            &state.sqlx_pool,
            RuntimeTaskResolveRequest {
                bear_id: session.bear_id,
                profile: BearProfile::Pair,
                user_id: Some(session.user_id),
                conversation_id: conversation_runtime_id.clone(),
                client_session_id: session.client_session_id.clone(),
                cached_activity_plan_projection: den_runtime::native_runtime::native_client_session_cached_activity_plan_projection(
                    &conversation_runtime_id,
                    &session.client_session_id,
                ),
            },
        )
        .await
        .map_err(|error| match error {
            DenError::Database(message) => CustomError::Database(format!(
                "resolve Pair runtime task context for BearWire session.state: bear_id={}, client_session_id={}, conversation_id={}: {message}",
                session.bear_id, session.client_session_id, conversation_runtime_id
            )),
            DenError::DatabaseUnavailable(message) => CustomError::DatabaseUnavailable(format!(
                "resolve Pair runtime task context for BearWire session.state: bear_id={}, client_session_id={}, conversation_id={}: {message}",
                session.bear_id, session.client_session_id, conversation_runtime_id
            )),
            error => error.into(),
        })?;
        Some(context)
    } else {
        None
    };
    let current_task = runtime_task_context
        .as_ref()
        .and_then(pair_current_task_projection);
    let active_activity_plan = runtime_task_context.as_ref().and_then(|focus| {
        focus.active_activity_plan().cloned().map(|plan| {
            active_activity_plan_projection(plan, focus.source.as_str(), current_task.clone())
        })
    });
    let active_docket_execution = if work_enabled {
        PgDocketService::from_pool(&state.sqlx_pool)
            .get_active_execution_session(
                session.bear_id,
                BearProfile::Pair,
                active_docket_execution_lookup_for_session(
                    &conversation_runtime_id,
                    &session.client_session_id,
                ),
            )
            .await?
            .map(active_docket_execution_projection)
    } else {
        None
    };
    let runtime_state = den_runtime::native_runtime::native_client_session_runtime_state(
        &conversation_runtime_id,
        &session.client_session_id,
    );
    let open_obligations = turn_obligations::open_client_obligations_for_session(
        &state.sqlx_pool,
        &session.client_session_id,
    )
    .await?
    .into_iter()
    .map(|obligation| {
        json!({
            "id": obligation.id,
            "run_id": obligation.run_id,
            "kind": obligation.kind,
            "expected_responder_action": obligation.expected_responder_action,
            "tool_call_id": obligation.tool_call_id,
            "permission_id": obligation.permission_id,
            "state": obligation.state,
            "turn_step_id": obligation.turn_step_id,
            "created_at": obligation.created_at,
            "updated_at": obligation.updated_at,
            "timeout_ms": obligation.timeout_ms(),
            "expires_at": obligation.expires_at(),
        })
    })
    .collect::<Vec<_>>();

    Ok(json!({
        "id": session.id,
        "user_id": session.user_id,
        "bear_id": session.bear_id,
        "bear_slug": session.bear_slug,
        "client_session_id": session.client_session_id,
        "runtime_session_id": session.runtime_session_id,
        "conversation_id": session.conversation_id,
        "resolved_conversation_id": session.resolved_conversation_id,
        "client": session.client,
        "cwd": session.cwd,
        "adapter_environment": session.adapter_environment,
        "current_mode": session.current_mode,
        "conversation_title": session.conversation_title,
        "conversation_title_updated_at": session.conversation_title_updated_at,
        "conversation_title_synced_at": session.conversation_title_synced_at,
        "closed_at": session.closed_at,
        "archived_at": session.archived_at,
        "created_at": session.created_at,
        "updated_at": session.updated_at,
        "context_budget": latest_context_budget,
        "current_task": current_task,
        "diagnostics": {
            "trusted_workspace": trusted_workspace,
            "runtime_conversation_id": conversation_runtime_id,
            "runtime_session_live": runtime_session_live,
            "runtime_state": runtime_state,
            "active_activity_plan": active_activity_plan,
            "active_docket_execution": active_docket_execution,
            "open_obligations": open_obligations,
        }
    }))
}

fn pair_current_task_projection(
    context: &den_runtime::runtime::task_context::RuntimeTaskContext,
) -> Option<Value> {
    let task_id = context.current_task_id?;
    if !matches!(
        context.source,
        den_runtime::runtime::task_context::RuntimeTaskSource::SessionCurrentTask
            | den_runtime::runtime::task_context::RuntimeTaskSource::DurableDocketExecution
    ) {
        return None;
    }
    let item = context
        .active_activity_plan()?
        .current_item
        .as_ref()
        .filter(|item| item.id == task_id.to_string())?;
    Some(json!({
        "id": item.id,
        "title": item.title,
        "summary": item.summary,
        "status": item.status,
        "source_ref": item.source_ref,
    }))
}

fn active_docket_execution_projection(execution: den_docket::DocketExecutionSessionRow) -> Value {
    json!({
        "schema": "den.docket.active_execution.v1",
        "source": "docket_execution_session",
        "id": execution.id,
        "owner_profile": execution.owner_profile,
        "session_id": execution.session_id,
        "source_conversation_id": execution.source_conversation_id,
        "source_client_session_id": execution.source_client_session_id,
        "job_id": execution.job_id,
        "run_id": execution.run_id,
        "task_id": execution.task_id,
        "state": execution.state,
        "updated_at": execution.updated_at,
    })
}

fn active_activity_plan_projection(
    plan: den_docket::TaskListProjection,
    source: &str,
    current_task: Option<Value>,
) -> Value {
    let current_item_id = plan.current_item.as_ref().map(|item| item.id.clone());
    json!({
        "schema": "den.acp_plan_projection.v1",
        "source": source,
        "projection": "flat_current_level",
        "id": plan.id,
        "title": plan.title,
        "status": plan.status,
        "version": plan.version,
        "current_item_id": current_item_id,
        "current_task": current_task,
        "items": plan.items.into_iter().map(|item| {
            let selection = (current_item_id.as_deref() == Some(item.id.as_str()))
                .then_some("current");
            json!({
                "id": item.id,
                "title": item.title,
                "summary": item.summary,
                "status": item.status,
                "selection": selection,
                "blocked_reason": item.blocked_reason,
                "source_ref": item.source_ref,
                "sync_state": item.sync_state,
            })
        }).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use den_docket::{TaskListProjection, TaskListSourceRef, TaskListSyncState};
    use den_runtime::runtime::task_context::{RuntimeTaskContext, RuntimeTaskSource};
    use sqlx::types::time::OffsetDateTime;
    use uuid::Uuid;

    fn session_current_task_context(task_id: Uuid) -> RuntimeTaskContext {
        let item = TaskListItem {
            id: task_id.to_string(),
            title: "Selected task".to_string(),
            summary: Some("Current Pair task".to_string()),
            status: TaskListItemStatus::Pending,
            blocked_reason: None,
            source_ref: TaskListSourceRef::local(vec![]),
            sync_state: TaskListSyncState::CheckedOut,
        };
        RuntimeTaskContext {
            source: RuntimeTaskSource::SessionCurrentTask,
            current_task_id: Some(task_id),
            cached_activity_plan_projection: Some(TaskListProjection {
                id: Uuid::new_v4(),
                bear_id: Uuid::new_v4(),
                title: "Session tasks".to_string(),
                summary: String::new(),
                owner_profile: "pair".to_string(),
                visibility: "private_to_profile".to_string(),
                status: "active".to_string(),
                version: 1,
                source_ref: TaskListSourceRef::local(vec![]),
                items: vec![item.clone()],
                current_item: Some(item),
                source_conversation_id: None,
                source_client_session_id: None,
                handoff_intent_path: None,
                handoff_task_id: None,
                created_at: OffsetDateTime::UNIX_EPOCH,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            }),
        }
    }

    #[test]
    fn pair_current_task_projects_session_and_durable_execution_focus() {
        let task_id = Uuid::new_v4();
        let projected = pair_current_task_projection(&session_current_task_context(task_id))
            .expect("session-selected task should project");
        assert_eq!(projected["id"], task_id.to_string());
        assert_eq!(projected["title"], "Selected task");

        let plan = session_current_task_context(task_id)
            .active_activity_plan()
            .cloned()
            .expect("session task plan");
        let acp_projection = active_activity_plan_projection(
            plan,
            RuntimeTaskSource::SessionCurrentTask.as_str(),
            Some(projected),
        );
        assert_eq!(acp_projection["current_task"]["id"], task_id.to_string());
        assert_eq!(acp_projection["status"], "active");
        assert_eq!(acp_projection["items"][0]["status"], "pending");
        assert_eq!(acp_projection["items"][0]["selection"], "current");

        let mut no_selection = session_current_task_context(task_id);
        no_selection.current_task_id = None;
        assert!(pair_current_task_projection(&no_selection).is_none());

        let no_selection_plan = no_selection
            .active_activity_plan()
            .cloned()
            .expect("session task plan");
        let acp_without_selection = active_activity_plan_projection(
            no_selection_plan,
            RuntimeTaskSource::SessionCurrentTask.as_str(),
            None,
        );
        assert!(acp_without_selection["current_task"].is_null());

        let mut durable_execution = session_current_task_context(task_id);
        durable_execution.source = RuntimeTaskSource::DurableDocketExecution;
        let projected = pair_current_task_projection(&durable_execution)
            .expect("scheduler-selected durable task should project");
        assert_eq!(projected["id"], task_id.to_string());
    }
}

pub(crate) async fn session_open_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionOpenRequest = parse_params(params)?;
    let session_id = request.session_id;
    let existing = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?;
    let client = request.client.unwrap_or_else(|| DEFAULT_CLIENT.to_string());
    let conversation_id = request
        .conversation_id
        .or_else(|| {
            existing
                .as_ref()
                .map(|session| session.conversation_id.clone())
        })
        .unwrap_or_else(|| format!("new-acp-{client}-{}", uuid::Uuid::new_v4().simple()));
    let resolved_conversation_id = existing
        .as_ref()
        .and_then(|session| session.resolved_conversation_id.clone());
    let runtime_session_id = request
        .runtime_session_id
        .or_else(|| {
            existing
                .as_ref()
                .map(|session| session.runtime_session_id.clone())
        })
        .unwrap_or_else(|| format!("bearwire:{}:{}", bear.id, session_id));
    let cwd = request.cwd;
    let current_mode = request
        .mode
        .as_deref()
        .map(client_sessions::ClientSessionMode::try_from_storage)
        .transpose()?;
    let client_context = request.client_context;
    client_sessions::upsert_session(
        &state.sqlx_pool,
        client_sessions::UpsertClientSession {
            user_id,
            bear_id: bear.id,
            bear_slug: bear.slug.clone(),
            client_session_id: session_id.clone(),
            runtime_session_id,
            conversation_id,
            resolved_conversation_id,
            client,
            cwd,
            current_mode,
        },
    )
    .await?;
    let reconnected =
        den_docket::work_runs::reconnect_attached_work_run(&state.sqlx_pool, &session_id)
            .await?
            .is_some();
    if let Some(client_context) = client_context.as_ref() {
        client_sessions::update_adapter_environment(
            &state.sqlx_pool,
            user_id,
            bear.id,
            &session_id,
            client_context,
        )
        .await?;
    }
    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?;
    let mut event = BearWireEvent::ephemeral(
        "session.opened",
        json!({
            "session_id": session_id,
            "bear_slug": bear.slug,
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    let persisted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;
    Ok(json!({
        "ok": true,
        "session": session,
        "event_sequence": persisted.sequence_no,
        "attached_work_reconnected": reconnected,
    }))
}

pub(crate) async fn session_compact_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionIdRequest = parse_params(params)?;
    let session_id = request.session_id;
    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let conversation_id = session
        .resolved_conversation_id
        .as_deref()
        .unwrap_or(&session.conversation_id);
    let state_result = prepare_turn_compaction(
        &state.sqlx_pool,
        &state.config,
        bear.id,
        conversation_id,
        BearProfile::Pair,
        TurnCompactionTrigger::Manual,
    )
    .await?;

    let compacted = state_result
        .as_ref()
        .is_some_and(|state| state.compacted_seq_cutoff.is_some());
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "conversation_id": conversation_id,
        "compact_result": {
            "status": if compacted { "applied" } else { "skipped" },
            "reason": "bearwire_manual",
            "compacted_seq_cutoff": state_result.as_ref().and_then(|state| state.compacted_seq_cutoff),
            "group_count": state_result.as_ref().map(|state| state.groups.len()).unwrap_or(0),
        }
    }))
}

pub(crate) async fn session_close_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionIdRequest = parse_params(params)?;
    let session_id = request.session_id;
    let Some(session) = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    else {
        return Ok(json!({ "ok": true, "closed": false, "session_id": session_id }));
    };
    let reflection_payload =
        match reflect_pair_session(&state.sqlx_pool, state, &session, "session_close").await {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(
                    bear_id = %bear.id,
                    session_id = %session_id,
                    error = %error,
                    "pair reflection failed during session close"
                );
                json!({
                    "status": "failed_open",
                    "error": error.to_string(),
                })
            }
        };
    client_sessions::mark_closed(&state.sqlx_pool, session.id).await?;
    let disconnected = den_docket::work_runs::disconnect_attached_work_run(
        &state.sqlx_pool,
        &session_id,
        den_docket::work_runs::ATTACHED_DISCONNECT_TIMEOUT,
    )
    .await?
    .is_some();
    let mut event = BearWireEvent::ephemeral(
        "session.closed",
        json!({
            "session_id": session_id,
            "bear_slug": bear.slug,
            "pair_reflection": reflection_payload,
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    let persisted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;
    Ok(json!({
        "ok": true,
        "closed": true,
        "session_id": session_id,
        "event_sequence": persisted.sequence_no,
        "pair_reflection": reflection_payload,
        "attached_work_disconnected": disconnected,
    }))
}

pub(crate) async fn session_state_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let request: SessionStateRequest = parse_params(params)?;
    let Some(bear_slug) = request
        .bear_slug
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(json!({
            "status": "available",
            "note": "Provide bear_slug and optional session_id for authenticated BearWire session state.",
            "params": params,
        }));
    };
    let user_id = authenticate_for_bear_slug(state, headers, bear_slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, bear_slug)
        .await?
        .ok_or_else(|| CustomError::NotFound("Bear not found or token lacks access".to_string()))?;
    if let Some(session_id) = request
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let session = client_sessions::find_for_user_bear_session(
            &state.sqlx_pool,
            user_id,
            bear_slug,
            session_id,
        )
        .await?;
        return Ok(json!({
            "kind": "single",
            "bear_slug": bear_slug,
            "session": match session {
                Some(session) => Some(session_state_payload(state, session, bear.work_enabled).await?),
                None => None,
            },
        }));
    }

    let include_closed = request.include_closed.unwrap_or(false);
    let limit = request.limit.unwrap_or(50).clamp(1, 100);
    let sessions = client_sessions::list_for_user_bear(
        &state.sqlx_pool,
        client_sessions::SessionListParams {
            user_id,
            bear_slug,
            include_closed,
            cwd_filter: None,
            limit,
            cursor_updated_at: None,
            cursor_id: None,
        },
    )
    .await?;
    let mut sessions_payload = Vec::with_capacity(sessions.len());
    for session in sessions {
        sessions_payload.push(session_state_payload(state, session, bear.work_enabled).await?);
    }
    Ok(json!({
        "kind": "list",
        "bear_slug": bear_slug,
        "sessions": sessions_payload,
    }))
}

pub(crate) async fn session_current_task_selection_request_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionCurrentTaskSelectionRequest = parse_params(params)?;
    let task_id = uuid::Uuid::parse_str(&request.task_id)
        .map_err(|_| CustomError::ValidationError("task_id must be a UUID".to_string()))?;
    let title = preview_pair_current_task_selection(
        &state.sqlx_pool,
        user_id,
        bear.id,
        &request.session_id,
        task_id,
    )
    .await?;
    Ok(
        json!({"ok": true, "confirmation_required": true, "session_id": request.session_id, "task_id": task_id, "title": title}),
    )
}

pub(crate) async fn session_current_task_select_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionCurrentTaskSelectionRequest = parse_params(params)?;
    let task_id = uuid::Uuid::parse_str(&request.task_id)
        .map_err(|_| CustomError::ValidationError("task_id must be a UUID".to_string()))?;
    if !bear.work_enabled {
        return Err(CustomError::ValidationError(
            "Pair task controls are disabled".to_string(),
        ));
    }
    let result = select_pair_current_task(
        &state.sqlx_pool,
        user_id,
        bear.id,
        &request.session_id,
        Some(task_id),
    )
    .await?;
    Ok(
        json!({"ok": true, "session_id": request.session_id, "current_task_id": task_id, "title": result.title, "task_list": result.task_list}),
    )
}

pub(crate) async fn session_current_task_start_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionCurrentTaskStartRequest = parse_params(params)?;
    if !bear.work_enabled {
        return Err(CustomError::ValidationError(
            "Pair task controls are disabled".to_string(),
        ));
    }
    let session = client_sessions::find_for_user_bear_session_id(
        &state.sqlx_pool,
        user_id,
        bear.id,
        &request.session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("client session not found".to_string()))?;
    let task_id = session.current_task_id.ok_or_else(|| {
        CustomError::ValidationError(
            "no current Pair task is selected for this session".to_string(),
        )
    })?;
    let title = preview_pair_current_task_selection(
        &state.sqlx_pool,
        user_id,
        bear.id,
        &request.session_id,
        task_id,
    )
    .await?;

    if let Some(run) =
        den_runtime::turn_runs::active_run_for_session(&state.sqlx_pool, &request.session_id)
            .await?
            .filter(|run| run.bear_id == bear.id && run.user_id == user_id)
    {
        if run.state == "continuing" {
            // A continuing run has no live stream after process loss. Return the
            // same durable run to retryable state; creating a successor here
            // would violate the one-active-run-per-session invariant.
            let recovered = den_runtime::turn_runs::begin_claimed_run_continuation(
                &state.sqlx_pool,
                &run.run_id,
            )
            .await?
            .ok_or_else(|| {
                CustomError::ValidationError(
                    "continuation recovery was claimed concurrently".to_string(),
                )
            })?;
            den_runtime::agent_loop::record_loop_control_decision(
                &state.sqlx_pool,
                LoopControlLedgerInput {
                    run_id: recovered.run_id.clone(),
                    turn_step_id: None,
                    conversation_message_id: None,
                    decision_id: format!("budget-slice-recovery:{}", recovered.run_id),
                    decision_kind: LoopControlDecisionKind::BudgetSliceRecovery,
                    control_level: "standard".to_string(),
                    reason: Some("process_abandoned_continuation".to_string()),
                    orientation_kind: Some("task_oriented".to_string()),
                    checkpoint_id: None,
                    related_task_list_id: None,
                    related_task_item_id: Some(task_id.to_string()),
                    related_docket_job_id: None,
                    related_docket_task_id: Some(task_id),
                    evidence_refs: vec![LedgerEvidenceRef {
                        kind: "turn_run_state".to_string(),
                        id: "continuing_to_running".to_string(),
                    }],
                    decision: json!({ "action": "recover", "same_run": true }),
                },
            )
            .await?;
            let attempt = start_pair_execution_attempt(
                &state.sqlx_pool,
                bear.id,
                task_id,
                &request.session_id,
                &recovered.run_id,
            )
            .await?;
            return Ok(json!({
                "ok": true,
                "started": false,
                "recovered": true,
                "reused": true,
                "recovered_run_id": recovered.run_id,
                "run_id": recovered.run_id,
                "session_id": request.session_id,
                "task_id": task_id,
                "state": recovered.state,
                "execution_attempt_id": attempt.id,
                "fence_epoch": attempt.fence_epoch,
            }));
        }
        let attempt = start_pair_execution_attempt(
            &state.sqlx_pool,
            bear.id,
            task_id,
            &request.session_id,
            &run.run_id,
        )
        .await?;
        return Ok(json!({
            "ok": true,
            "started": false,
            "reused": true,
            "run_id": run.run_id,
            "session_id": request.session_id,
            "task_id": task_id,
            "state": run.state,
            "execution_attempt_id": attempt.id,
            "fence_epoch": attempt.fence_epoch,
        }));
    }

    // ponytail: this delegates to the established run.start lifecycle so task-start
    // cannot drift from Pair stream/event behavior. Concurrent starts can still race
    // between the active-run read and run.start; make run creation session-unique if
    // clients require concurrent idempotency rather than retry idempotency.
    let mut start_params = serde_json::Map::new();
    start_params.insert("bear_slug".to_string(), json!(bear.slug));
    start_params.insert("session_id".to_string(), json!(request.session_id));
    start_params.insert(
        "prompt".to_string(),
        json!(format!("Start working on the selected task: {title}")),
    );
    start_params.insert("client".to_string(), json!(session.client));
    start_params.insert(
        "conversation_id".to_string(),
        json!(session.conversation_id),
    );
    if let Some(cwd) = session.cwd {
        start_params.insert("cwd".to_string(), json!(cwd));
    }
    if let Some(client_context) = session.adapter_environment {
        start_params.insert("client_context".to_string(), client_context);
    }
    let request: RunStartRequest = serde_json::from_value(Value::Object(start_params))
        .map_err(|err| CustomError::ValidationError(format!("invalid task start params: {err}")))?;
    let task_session_id = request.session_id.clone();
    let result =
        super::run::run_start_for_pair_task(state, request, user_id, bear.clone(), task_id).await?;
    let run_id = result["run_id"].as_str().ok_or_else(|| {
        CustomError::ValidationError("run.start returned a non-string run_id".to_string())
    })?;
    let execution_attempt_id = result["execution_attempt_id"].clone();
    let fence_epoch = result["fence_epoch"].clone();
    Ok(json!({
        "ok": true,
        "started": true,
        "reused": false,
        "run_id": run_id,
        "session_id": task_session_id,
        "task_id": task_id,
        "state": result["state"].clone(),
        "event_sequence": result["event_sequence"].clone(),
        "execution_attempt_id": execution_attempt_id,
        "fence_epoch": fence_epoch,
    }))
}

async fn start_pair_execution_attempt(
    pool: &PgPool,
    bear_id: uuid::Uuid,
    task_id: uuid::Uuid,
    session_id: &str,
    run_id: &str,
) -> Result<den_docket::DocketExecutionAttemptRow, CustomError> {
    let authorization_key = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, run_id.as_bytes());
    let service = PgDocketService::from_pool(pool);
    let attempt = service
        .authorize_execution_attempt(DocketExecutionAttemptAuthorize {
            bear_id,
            task_id,
            owner: DocketExecutionAttemptOwner::Pair {
                session_id: session_id.to_string(),
                pair_run_id: run_id.to_string(),
            },
            authorization_key,
        })
        .await?;
    Ok(service
        .start_execution_attempt(DocketExecutionAttemptStart {
            attempt_id: attempt.id,
            fence_epoch: attempt.fence_epoch,
        })
        .await?)
}

pub(crate) async fn session_current_task_clear_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionCurrentTaskClearRequest = parse_params(params)?;
    if !bear.work_enabled {
        return Err(CustomError::ValidationError(
            "Pair task controls are disabled".to_string(),
        ));
    }
    let result = select_pair_current_task(
        &state.sqlx_pool,
        user_id,
        bear.id,
        &request.session_id,
        None,
    )
    .await?;
    Ok(
        json!({"ok": true, "session_id": request.session_id, "current_task_id": Value::Null, "task_list": result.task_list}),
    )
}

async fn session_model_payload(
    state: &DenState,
    user_id: i32,
    bear: &den_service::bears::Bear,
    session_id: &str,
) -> Result<Value, CustomError> {
    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let conversation_id = session
        .resolved_conversation_id
        .as_deref()
        .unwrap_or(&session.conversation_id);
    let view = den_service::model_selection::load_conversation_model_selection_view(
        &state.sqlx_pool,
        bear,
        user_id,
        BearProfile::Pair,
        state.config.default_llm_model.as_str(),
        conversation_id,
        Some(&session.client_session_id),
        true,
    )
    .await?;
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "conversation_id": conversation_id,
        "selection_mode": view.selection_mode,
        "requested_model": view.requested_model,
        "selected_model": view.selected_model,
        "effective_model": view.effective_model,
        "model_options": view.model_options,
    }))
}

pub(crate) async fn session_model_get_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionIdRequest = parse_params(params)?;
    let session_id = request.session_id;
    session_model_payload(state, user_id, &bear, &session_id).await
}

pub(crate) async fn session_model_set_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: SessionModelSetRequest = parse_params(params)?;
    let session_id = request.session_id;
    let mode = request.selection_mode.unwrap_or_else(|| "auto".to_string());
    let requested_model = request.model;
    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let conversation_id = session
        .resolved_conversation_id
        .as_deref()
        .unwrap_or(&session.conversation_id);
    let conversation = den_service::conversation::persistence::ensure_conversation_for_external_id(
        &state.sqlx_pool,
        bear.id,
        Some(user_id),
        conversation_id,
        Some(&session.client_session_id),
        None,
    )
    .await?;
    let model_state = den_service::model_selection::apply_conversation_model_selection(
        &state.sqlx_pool,
        conversation.id,
        &mode,
        requested_model.as_deref(),
        "acp_selected",
        "inherit_stance_or_bear_default",
    )
    .await
    .map_err(CustomError::from)?;

    let mut event = BearWireEvent::ephemeral(
        "model.selection.changed",
        json!({
            "session_id": session_id,
            "conversation_id": conversation_id,
            "selection_mode": model_state.selection_mode,
            "selected_model": model_state.selected_model.or(model_state.requested_model),
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    let persisted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;

    let mut payload = session_model_payload(state, user_id, &bear, &session_id).await?;
    if let Some(object) = payload.as_object_mut() {
        object.insert("event_sequence".to_string(), json!(persisted.sequence_no));
    }
    Ok(payload)
}
