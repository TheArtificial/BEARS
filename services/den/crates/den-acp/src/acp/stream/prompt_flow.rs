use axum::http::StatusCode;
use uuid::Uuid;

use crate::acp::client_tool_advertisement::client_tool_descriptors_for_client_context;
use crate::{
    acp::{
        acp_error_status_message, authenticate_acp_code_token_with_auth,
        history::acp_auto_title_instruction,
        paths::require_absolute_cwd,
        prompt_context::{
            acp_direct_tool_prompt_context_with_activity, acp_plan_mode_prompt_context,
        },
        requested_mode_from_prompt, resolve_acp_turn_context,
        stream::orchestration::{build_acp_sse_response, build_acp_stream_setup},
        AcpPromptRequest,
    },
    core::{
        acp_runtime::{
            canonical_acp_conversation_id_for_session, require_pair_runtime_binding,
            AcpConversationService,
        },
        armature_tokens,
        docket::{DocketExecutionLookup, DocketService, PgDocketService},
        work_plans::{self, WorkPlanLookup},
    },
    service::DenState,
};
use den_http::errors::CustomError;
use den_oauth::auth::{self, ApiError};
use den_service::{
    client_sessions::{self, UpsertClientSession},
    bears::{db as bears_db, BearProfile},
    conversation::events::{
        persist_canonical_conversation_record, CanonicalConversationRecord,
        ConversationEventProvenance, ConversationPersistenceContext,
    },
};
use den_runtime::{
    plan_mode,
};
use den_service::bears::prompt_fragments::{
    render_turn_fragment, repository_prompt_fragment_registry,
};

fn docket_execution_gate_for_acp(policy_mode_label: &str) -> serde_json::Value {
    let is_write = policy_mode_label.eq_ignore_ascii_case("write");
    serde_json::json!({
        "state": if is_write { "open" } else { "blocked" },
        "reason": if is_write { "write_enabled" } else { "read_only_surface" },
        "required_action": if is_write { "none" } else { "switch_work_surface_to_write" },
        "can_mutate_workspace": is_write,
        "can_run_processes": is_write,
        "can_use_browser": is_write,
    })
}

async fn acp_docket_execution_prompt_context(
    state: &DenState,
    bear_id: Uuid,
    session_id: &str,
    policy_mode_label: &str,
) -> Result<String, CustomError> {
    let execution = PgDocketService::from_pool(&state.sqlx_pool)
        .get_active_execution_session(
            bear_id,
            BearProfile::Pair,
            DocketExecutionLookup {
                session_id: Some(session_id.to_string()),
                source_conversation_id: None,
                source_acp_session_id: Some(session_id.to_string()),
            },
        )
        .await
        .map_err(CustomError::from)?;
    let Some(execution) = execution else {
        return Ok(String::new());
    };
    let registry = repository_prompt_fragment_registry().map_err(CustomError::from)?;
    let fragment = registry
        .require("runtime_docket_execution_active")
        .map_err(CustomError::from)?;
    let body = render_turn_fragment(
        fragment,
        &serde_json::json!({
            "execution": {
                "id": execution.id,
                "state": execution.state,
                "job_id": execution.job_id,
                "run_id": execution.run_id,
                "task_id": execution.task_id,
                "session_id": execution.session_id,
                "owner_profile": execution.owner_profile,
                "source_acp_session_id": execution.source_acp_session_id,
                "source_conversation_id": execution.source_conversation_id,
                "surface": {
                    "kind": "armature",
                    "adapter": "acp",
                    "stance": "pair"
                },
                "permission": {
                    "source": "acp",
                    "mode_label": policy_mode_label
                },
                "gate": docket_execution_gate_for_acp(policy_mode_label),
            }
        }),
    )
    .map_err(CustomError::from)?;
    Ok(format!("\n\n<system-reminder>{body}</system-reminder>"))
}

pub(in crate::acp) async fn run_prompt_flow(
    state: DenState,
    slug: String,
    session_id: String,
    headers: axum::http::HeaderMap,
    body: AcpPromptRequest,
    request_id: Uuid,
) -> Result<Result<axum::response::Response, CustomError>, ApiError> {
    let slug = slug.trim().to_string();
    let token = auth::extract_bearer_token(&headers)?;
    let auth = authenticate_acp_code_token_with_auth(&state, &token, &slug)
        .await
        .map_err(|err| {
            let (status, code, message) = acp_error_status_message(&err);
            ApiError::new(status, code, message)
        })?;
    let user_id = auth.user_id;
    let tools_enabled = armature_tokens::scopes_contains(&auth.scopes, armature_tokens::armature_tools_scope());
    if !tools_enabled {
        tracing::info!(bear_slug = %slug, user_id = user_id, "Armature token lacks armature:tools; local client tools disabled for prompt");
    }
    let prompt = body.message.trim();
    if prompt.is_empty() {
        return Ok(Err(CustomError::ValidationError(
            "message must not be empty".to_string(),
        )));
    }

    let slug = slug.trim();
    if slug.is_empty() {
        return Ok(Err(CustomError::NotFound("bear not found".to_string())));
    }

    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, slug)
        .await
        .map_err(|err| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "database",
                err.to_string(),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "bear not found or you do not have access",
            )
        })?;

    let pair_runtime_binding =
        match require_pair_runtime_binding(&state.sqlx_pool, state.config.as_ref(), &bear).await {
            Ok(binding) => binding,
            Err(err) => return Ok(Err(err)),
        };

    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(Err(CustomError::ValidationError(
            "session_id must not be empty".to_string(),
        )));
    }

    let client = super::super::normalize_acp_client(body.client.as_deref());
    let cwd = require_absolute_cwd(body.client_context.get("cwd").and_then(|v| v.as_str()))
        .map_err(|err| {
            let (status, code, message) = acp_error_status_message(&err);
            ApiError::new(status, code, message)
        })?;
    let existing_session =
        client_sessions::find_for_user_bear_session(&state.sqlx_pool, user_id, &bear.slug, session_id)
            .await
            .map_err(|err| {
                ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "database",
                    err.to_string(),
                )
            })?;
    let is_new_session_binding = existing_session.is_none();
    let requested_initial_mode = match requested_mode_from_prompt(&body) {
        Ok(mode) => mode,
        Err(err) => return Ok(Err(err)),
    };
    let generated_conversation_id = super::super::new_acp_conversation_id(&client);
    let conversation_runtime = AcpConversationService::new(&state.sqlx_pool, state.config.as_ref());
    let (conversation_resolution, ensure_conversation_result) = conversation_runtime
        .ensure_prompt_conversation(
            den_protocol::EnsureConversationRequest {
                bear_id: bear.id,
                role: "pair".to_string(),
                client_session_id: session_id.to_string(),
                requested_selection: body.conversation_id.clone(),
                binding: pair_runtime_binding.clone(),
            },
            existing_session.as_ref(),
            generated_conversation_id,
        )
        .await
        .map_err(|err| {
            let (status, code, message) = acp_error_status_message(&err);
            ApiError::new(status, code, message)
        })?;
    if conversation_resolution.requires_belongs_to_bear_check {
        conversation_runtime
            .verify_conversation_access(
                bear.id,
                &pair_runtime_binding,
                &conversation_resolution.session_selection,
            )
            .await
            .map_err(|err| {
                let (status, code, message) = acp_error_status_message(&err);
                ApiError::new(status, code, message)
            })?;
    }
    if ensure_conversation_result.created {
        tracing::info!(
            %request_id,
            acp_session_id = %session_id,
            bear_id = %bear.id,
            pending_conversation_id = %conversation_resolution.session_selection,
            resolved_conversation_id = %ensure_conversation_result.conversation.id,
            "ACP created runtime conversation for explicit pending session selection"
        );
    }
    let canonical_conversation_id = canonical_acp_conversation_id_for_session(
        existing_session.as_ref(),
        &conversation_resolution,
    );
    let runtime_session_id = format!("acp-api-direct:{client}:{}:{session_id}", bear.id);
    client_sessions::upsert_session(
        &state.sqlx_pool,
        UpsertClientSession {
            user_id,
            bear_id: bear.id,
            bear_slug: bear.slug.clone(),
            client_session_id: session_id.to_string(),
            runtime_session_id: runtime_session_id.clone(),
            conversation_id: canonical_conversation_id.clone(),
            resolved_conversation_id: conversation_resolution
                .resolved_conversation
                .as_ref()
                .map(|conversation| conversation.id.clone()),
            client: client.clone(),
            cwd: Some(cwd.clone()),
            current_mode: if is_new_session_binding {
                requested_initial_mode.map(str::to_string)
            } else {
                None
            },
        },
    )
    .await
    .map_err(|err| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "database",
            err.to_string(),
        )
    })?;
    if is_new_session_binding {
        match requested_initial_mode {
            Some("plan") => {
                plan_mode::enter_plan_mode(
                    &state.sqlx_pool,
                    plan_mode::EnterPlanModeParams {
                        user_id,
                        bear_id: bear.id,
                        bear_slug: bear.slug.clone(),
                        client_session_id: session_id.to_string(),
                        reason: "Client selected ACP Plan mode before first prompt".to_string(),
                        requested_by: plan_mode::PlanModeRequestedBy::User,
                        previous_permission_mode: Some("ask".to_string()),
                    },
                )
                .await
                .map_err(|err| {
                    let (status, code, message) = acp_error_status_message(&err.into());
                    ApiError::new(status, code, message)
                })?;
                client_sessions::set_current_mode(
                    &state.sqlx_pool,
                    user_id,
                    bear.id,
                    session_id,
                    "plan",
                )
                .await
                .map_err(|err| {
                    let (status, code, message) = acp_error_status_message(&CustomError::from(err));
                    ApiError::new(status, code, message)
                })?;
            }
            Some("write") => {
                client_sessions::set_current_mode(
                    &state.sqlx_pool,
                    user_id,
                    bear.id,
                    session_id,
                    "write",
                )
                .await
                .map_err(|err| {
                    let (status, code, message) = acp_error_status_message(&CustomError::from(err));
                    ApiError::new(status, code, message)
                })?;
            }
            _ => {}
        }
    }
    tracing::info!(
        %request_id,
        acp_session_id = %session_id,
        bear_slug = %bear.slug,
        bear_id = %bear.id,
        role = "pair",
        runtime_binding_id = %pair_runtime_binding.binding_id,
        client = %client,
        cwd = %cwd,
        requested_conversation_id = body.conversation_id.as_deref().map(str::trim),
        conversation_id = %canonical_conversation_id,
        conversation_selection_source = %conversation_resolution.selection_source.as_str(),
        resolved_conversation_id = conversation_resolution
            .resolved_conversation
            .as_ref()
            .map(|conversation| conversation.id.as_str()),
        history_target = conversation_resolution
            .history_target
            .as_ref()
            .map(|conversation| conversation.id.as_str()),
        archive_target = conversation_resolution
            .archive_target
            .as_ref()
            .map(|conversation| conversation.id.as_str()),
        runtime_conversation_id = %conversation_resolution.upstream_target,
        "ACP gateway routing prompt to pair role via native runtime"
    );
    let active_plan_mode =
        plan_mode::active_for_session(&state.sqlx_pool, user_id, bear.id, session_id)
            .await
            .map_err(|err| {
                let (status, code, message) = acp_error_status_message(&err.into());
                ApiError::new(status, code, message)
            })?;
    let session_mode =
        client_sessions::find_for_user_bear_session(&state.sqlx_pool, user_id, &bear.slug, session_id)
            .await
            .map_err(|err| {
                ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "database",
                    err.to_string(),
                )
            })?
            .map(|session| session.current_mode)
            .unwrap_or_else(|| "ask".to_string());
    let synthetic_session_row = client_sessions::ClientSessionRow {
        id: Uuid::nil(),
        user_id,
        bear_id: bear.id,
        bear_slug: bear.slug.clone(),
        client_session_id: session_id.to_string(),
        runtime_session_id: "runtime-test".to_string(),
        conversation_id: canonical_conversation_id.clone(),
        resolved_conversation_id: existing_session
            .as_ref()
            .and_then(|session| session.resolved_conversation_id.clone())
            .or_else(|| {
                conversation_resolution
                    .resolved_conversation
                    .as_ref()
                    .map(|conversation| conversation.id.clone())
            }),
        client: client.clone(),
        cwd: Some(cwd.clone()),
        adapter_environment: None,
        current_mode: session_mode,
        conversation_title: client_sessions::find_for_user_bear_session(
            &state.sqlx_pool,
            user_id,
            &bear.slug,
            session_id,
        )
        .await
        .map_err(|err| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "database",
                err.to_string(),
            )
        })?
        .and_then(|session| session.conversation_title),
        conversation_title_updated_at: None,
        conversation_title_synced_at: None,
        closed_at: None,
        archived_at: None,
        created_at: time::OffsetDateTime::UNIX_EPOCH,
        updated_at: time::OffsetDateTime::UNIX_EPOCH,
    };
    let auto_title_guidance = acp_auto_title_instruction(&synthetic_session_row);
    let resolved_policy =
        resolve_acp_turn_context(&synthetic_session_row, active_plan_mode.as_ref(), None).policy;
    let plan_mode_context = acp_plan_mode_prompt_context(&state, bear.id, user_id, session_id)
        .await
        .map_err(|err| {
            let (status, code, message) = acp_error_status_message(&err);
            ApiError::new(status, code, message)
        })?;
    let docket_execution_context = acp_docket_execution_prompt_context(
        &state,
        bear.id,
        session_id,
        resolved_policy.mode_label,
    )
    .await
    .map_err(|err| {
        let (status, code, message) = acp_error_status_message(&err);
        ApiError::new(status, code, message)
    })?;
    let plan_mode_context = format!("{plan_mode_context}{docket_execution_context}");
    let current_activity_plan = PgDocketService::from_pool(&state.sqlx_pool)
        .get_visible_work_plan(
            bear.id,
            BearProfile::Pair,
            user_id,
            WorkPlanLookup {
                plan_id: None,
                source_conversation_id: None,
                source_acp_session_id: Some(session_id.to_string()),
            },
        )
        .await
        .map_err(CustomError::from)
        .map_err(|err| {
            let (status, code, message) = acp_error_status_message(&err);
            ApiError::new(status, code, message)
        })?;
    let (tool_prompt_context, prompt_memory_diagnostic) =
        acp_direct_tool_prompt_context_with_activity(
            &state,
            bear.id,
            session_id,
            &cwd,
            &body.client_context,
            tools_enabled,
            &resolved_policy,
            current_activity_plan.as_ref(),
            auto_title_guidance.as_deref(),
        )
        .await
        .map_err(|err| {
            let (status, code, message) = acp_error_status_message(&err);
            ApiError::new(status, code, message)
        })?;
    let merged_client_tool_descriptors = tools_enabled.then(|| {
        super::super::merge_acp_pair_tool_descriptors(
            client_tool_descriptors_for_client_context(
                &body.client_context,
                Some(&resolved_policy),
            ),
            true,
        )
    });
    let auto_title_tool_advertised = merged_client_tool_descriptors
        .as_ref()
        .and_then(|value| value.as_array())
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|name| {
                        name == den_core::tools::constants::DEN_CONVERSATION_SET_TITLE_PROVIDER
                    })
            })
        });
    tracing::info!(
        %request_id,
        acp_session_id = %session_id,
        auto_title_guidance_injected = auto_title_guidance.is_some(),
        auto_title_tool_advertised,
        current_conversation_title = synthetic_session_row.conversation_title.as_deref(),
        resolved_conversation_id = synthetic_session_row.resolved_conversation_id.as_deref(),
        conversation_id = %synthetic_session_row.conversation_id,
        "ACP auto-title prompt state"
    );
    let plans = current_activity_plan
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    let activity_context = work_plans::render_workboard_prompt_context(&plans);
    tracing::info!(
        %request_id,
        acp_session_id = %session_id,
        prompt_message_len = prompt.len(),
        plan_mode_context_len = plan_mode_context.len(),
        activity_context_len = activity_context.len(),
        docket_execution_context_len = docket_execution_context.len(),
        tool_prompt_context_len = tool_prompt_context.len(),
        prompt_has_trusted_mode_suffix = prompt.contains("Trusted ACP session mode this turn:"),
        prompt_has_system_reminder = prompt.contains("<system-reminder>"),
        "ACP prompt context assembly lengths"
    );
    if let Some(resolved_conversation_id) = conversation_resolution
        .resolved_conversation
        .as_ref()
        .map(|conversation| conversation.id.as_str())
        .filter(|id| *id == "default" || id.starts_with("conv-"))
    {
        let provenance = ConversationEventProvenance {
            source: "acp_prompt".to_string(),
            scope_id: session_id.to_string(),
        };
        let mut content_json = provenance.as_content_json("user_prompt");
        content_json["role"] = serde_json::json!("user");
        content_json["acp_session_id"] = serde_json::json!(session_id);
        content_json["client"] = serde_json::json!(client.clone());
        content_json["request_id"] = serde_json::json!(request_id.to_string());
        let record = CanonicalConversationRecord::visible_user_message(prompt, content_json, None);
        persist_canonical_conversation_record(
            &ConversationPersistenceContext {
                pool: state.sqlx_pool.clone(),
                bear_id: bear.id,
                user_id: Some(user_id),
                external_conversation_id: resolved_conversation_id.to_string(),
                source_session_id: Some(session_id.to_string()),
                request_id: Some(request_id.to_string()),
                persistence_scope_id: session_id.to_string(),
                skip_persistence: false,
            },
            &record,
        )
        .await
        .map_err(|err| {
            let (status, code, message) = acp_error_status_message(&CustomError::from(err));
            ApiError::new(status, code, message)
        })?;
    }

    let setup = build_acp_stream_setup(
        &state,
        user_id,
        &bear,
        session_id,
        &cwd,
        &body.client_context,
        &conversation_resolution,
        &current_activity_plan,
        &plan_mode_context,
        &activity_context,
        &tool_prompt_context,
        prompt_memory_diagnostic,
        prompt,
        request_id,
    )
    .await?;

    build_acp_sse_response(
        state,
        user_id,
        request_id,
        session_id,
        &bear,
        &client,
        prompt,
        &pair_runtime_binding,
        &conversation_resolution,
        &synthetic_session_row,
        &resolved_policy,
        &current_activity_plan,
        merged_client_tool_descriptors,
        setup,
    )
    .await
}
