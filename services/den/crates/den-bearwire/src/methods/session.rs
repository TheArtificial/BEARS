use axum::http::HeaderMap;
use den_docket::{
    DocketExecutionLookup, DocketService, PgDocketService, TaskListItem, TaskListItemStatus,
};
use serde_json::{json, Value};
use sqlx::PgPool;

use bearwire_protocol::{
    methods::{SessionIdRequest, SessionModelSetRequest, SessionOpenRequest, SessionStateRequest},
    wire::BearWireEvent,
};
use den_http::errors::CustomError;
use den_runtime::{
    bearwire_events,
    conversation_review::{
        ConversationReview, ConversationReviewFinding, ConversationReviewFindingDetail,
        ConversationReviewTrigger, FindingSource,
    },
    pair_reflection::create_pair_reflection_proposals_from_latest_summary,
    runtime::compaction::{prepare_turn_compaction, TurnCompactionState, TurnCompactionTrigger},
    turn_obligations,
};
use den_service::{
    bears::{db as bears_db, BearProfile},
    client_sessions, DenState,
};

use crate::auth::{authenticate_for_bear_slug, authenticated_bear};
use crate::methods::conversation::project_focus_title;
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
                tracing::warn!(session_id = %session.client_session_id, error = %error, "open-session pair reflection failed")
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
    let active_activity_plan = if work_enabled {
        den_runtime::native_runtime::native_client_session_active_activity_plan(
            &conversation_runtime_id,
            &session.client_session_id,
        )
        .map(active_activity_plan_projection)
    } else {
        None
    };
    let active_docket_execution = if work_enabled {
        PgDocketService::from_pool(&state.sqlx_pool)
            .get_active_execution_session(
                session.bear_id,
                BearProfile::Pair,
                DocketExecutionLookup {
                    session_id: Some(session.client_session_id.clone()),
                    source_conversation_id: None,
                    source_client_session_id: None,
                },
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

fn active_activity_plan_projection(plan: den_docket::TaskListProjection) -> Value {
    let current_item_id = plan.current_item.as_ref().map(|item| item.id.clone());
    json!({
        "schema": "den.acp_plan_projection.v1",
        "source": "native_agent_loop_active_activity_plan",
        "projection": "flat_current_level",
        "id": plan.id,
        "title": plan.title,
        "status": plan.status,
        "version": plan.version,
        "current_item_id": current_item_id,
        "items": plan.items.into_iter().map(|item| {
            let status = acp_plan_item_status(&item, current_item_id.as_deref());
            json!({
                "id": item.id,
                "title": item.title,
                "summary": item.summary,
                "status": status,
                "blocked_reason": item.blocked_reason,
                "source_ref": item.source_ref,
                "sync_state": item.sync_state,
            })
        }).collect::<Vec<_>>(),
    })
}

fn acp_plan_item_status(item: &TaskListItem, current_item_id: Option<&str>) -> &'static str {
    if current_item_id == Some(item.id.as_str()) {
        return "in_progress";
    }
    match item.status {
        TaskListItemStatus::Completed => "completed",
        TaskListItemStatus::InProgress => "in_progress",
        TaskListItemStatus::Pending
        | TaskListItemStatus::Blocked
        | TaskListItemStatus::Cancelled => "pending",
    }
}

fn normalized_mode(value: &str) -> &str {
    value.trim()
}

fn mode_changed(
    existing: Option<&den_service::client_sessions::ClientSessionRow>,
    requested: Option<&str>,
) -> bool {
    let Some(requested) = requested
        .map(normalized_mode)
        .filter(|mode| !mode.is_empty())
    else {
        return false;
    };
    existing
        .map(|session| normalized_mode(&session.current_mode) != requested)
        .unwrap_or(false)
}

async fn clear_focus_for_mode_change(
    state: &DenState,
    bear_id: uuid::Uuid,
    existing: Option<&den_service::client_sessions::ClientSessionRow>,
    requested_mode: Option<&str>,
) -> Result<u64, CustomError> {
    if !mode_changed(existing, requested_mode) {
        return Ok(0);
    }
    let Some(existing) = existing else {
        return Ok(0);
    };
    let conversation_id = existing
        .resolved_conversation_id
        .clone()
        .or_else(|| Some(existing.conversation_id.clone()));
    Ok(PgDocketService::from_pool(&state.sqlx_pool)
        .clear_active_execution_sessions(
            bear_id,
            DocketExecutionLookup {
                source_conversation_id: conversation_id,
                session_id: None,
                source_client_session_id: Some(existing.client_session_id.clone()),
            },
        )
        .await?)
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
    let current_mode = request.mode;
    let cleared_focus_count =
        clear_focus_for_mode_change(state, bear.id, existing.as_ref(), current_mode.as_deref())
            .await?;
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
    if cleared_focus_count > 0 {
        if let Some(session) = session.as_ref() {
            let title = project_focus_title(session.conversation_title.clone(), false);
            if title.is_some() || session.conversation_title_updated_at.is_some() {
                let mut update = BearWireEvent::ephemeral(
                    "session_info_update",
                    json!({
                        "title": title,
                        "updated_at": session
                            .conversation_title_updated_at
                            .unwrap_or(session.updated_at)
                            .to_string(),
                    }),
                );
                update.bear_id = Some(bear.id.to_string());
                update.human_id = Some(user_id.to_string());
                update.session_id = Some(session_id.clone());
                let _ = bearwire_events::append_bearwire_event(
                    &state.sqlx_pool,
                    &session_id,
                    Some(bear.id),
                    Some(user_id),
                    update,
                )
                .await;
            }
        }
    }
    let mut event = BearWireEvent::ephemeral(
        "session.opened",
        json!({
            "session_id": session_id,
            "bear_slug": bear.slug,
            "cleared_focus_count": cleared_focus_count,
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
        "cleared_focus_count": cleared_focus_count,
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
