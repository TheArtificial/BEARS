use sqlx::PgPool;
use uuid::Uuid;

use den_http::errors::CustomError;
use den_runtime::{
    acp_events::AcpGatewayEvent,
    acp_sessions,
    conversation_persistence::PersistedConversationMessage,
    agent_assist::sanitize_visible_transcript_text,
    runtime_compaction::{
            choose_compaction_decision, semantic_groups_from_runtime_messages,
            RuntimeCompactionDecision, RuntimeCompactionPolicy,
        },
    runtime_compaction_observability::{
            build_compaction_applied_event, build_compaction_skipped_event,
            RuntimeCompactionEvent,
        },
    runtime_conversations::{
            RuntimeCompactionTriggerKind, RuntimeIterativeSummary, RuntimeSemanticGroup,
        },
};

use super::{format_acp_session_timestamp, AcpConversationHistoryMessage};

pub(crate) fn normalize_acp_conversation_id(raw: Option<&str>) -> Result<String, CustomError> {
    let s = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    if s == "default" {
        return Ok("default".to_string());
    }
    let ok = (s.starts_with("conv-") || s.starts_with("new-"))
        && s.len() > 8
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(s.to_string())
    } else {
        Err(CustomError::ValidationError(format!(
            "invalid conversation_id (expected 'default', a runtime conv- id, or a pending new- id): {s}"
        )))
    }
}

fn runtime_messages_top_array(v: &serde_json::Value) -> &[serde_json::Value] {
    if let Some(a) = v.as_array() {
        return a.as_slice();
    }
    if let Some(a) = v.get("messages").and_then(|x| x.as_array()) {
        return a.as_slice();
    }
    if let Some(a) = v.get("data").and_then(|x| x.as_array()) {
        return a.as_slice();
    }
    if let Some(a) = v.get("items").and_then(|x| x.as_array()) {
        return a.as_slice();
    }
    &[]
}

fn runtime_inner_for_acp_history(msg: &serde_json::Value) -> &serde_json::Value {
    match msg.get("contents") {
        Some(c) if c.get("message_type").is_some() => c,
        _ => msg,
    }
}

pub(crate) fn runtime_messages_for_compaction(
    body: &serde_json::Value,
) -> Vec<serde_json::Value> {
    runtime_messages_top_array(body)
        .iter()
        .map(runtime_inner_for_acp_history)
        .cloned()
        .collect()
}

pub(crate) fn runtime_semantic_groups_for_compaction(
    body: &serde_json::Value,
) -> Vec<RuntimeSemanticGroup> {
    let messages = runtime_messages_for_compaction(body);
    semantic_groups_from_runtime_messages(&messages)
}

pub(crate) fn runtime_iterative_summary_for_compaction(
    body: &serde_json::Value,
) -> RuntimeIterativeSummary {
    let groups = runtime_semantic_groups_for_compaction(body);
    build_iterative_summary_from_groups(&groups)
}

pub(crate) fn default_runtime_compaction_policy() -> RuntimeCompactionPolicy {
    RuntimeCompactionPolicy {
        policy_version: "acp-history-v1".to_string(),
        protected_recent_group_count: 3,
        max_groups_before_compaction: 6,
        max_transcript_chars: 20_000,
    }
}

pub(crate) fn runtime_compaction_decision_for_history(
    body: &serde_json::Value,
    trigger: RuntimeCompactionTriggerKind,
) -> Option<RuntimeCompactionDecision> {
    let groups = runtime_semantic_groups_for_compaction(body);
    let policy = default_runtime_compaction_policy();
    choose_compaction_decision(&groups, trigger, &policy)
}

pub(crate) fn runtime_compaction_event_for_history(
    conversation_id: &str,
    body: &serde_json::Value,
    trigger: RuntimeCompactionTriggerKind,
) -> RuntimeCompactionEvent {
    let policy = default_runtime_compaction_policy();
    match runtime_compaction_decision_for_history(body, trigger.clone()) {
        Some(decision) => {
            let artifact = den_runtime::runtime_compaction::artifact_ref_from_decision(
                format!("{conversation_id}:{}-{}", decision.selected_group_start, decision.selected_group_end),
                &decision,
                &policy,
            );
            build_compaction_applied_event(conversation_id.to_string(), &decision, &policy, artifact)
        }
        None => build_compaction_skipped_event(
            conversation_id.to_string(),
            trigger,
            &policy,
            "no eligible history groups outside protected floors",
        ),
    }
}

fn build_iterative_summary_from_groups(groups: &[RuntimeSemanticGroup]) -> RuntimeIterativeSummary {
    let mut summary = RuntimeIterativeSummary::default();
    for group in groups {
        let label = format!(
            "{:?}:{}:{}",
            group.kind,
            group.start_message_id.as_deref().unwrap_or("start"),
            group.end_message_id.as_deref().unwrap_or("end")
        );
        match group.kind {
            den_runtime::runtime_conversations::RuntimeSemanticGroupKind::UserTurn => {
                push_unique_summary_value(&mut summary.active_user_goals, label);
            }
            den_runtime::runtime_conversations::RuntimeSemanticGroupKind::AssistantReply => {
                push_unique_summary_value(&mut summary.unresolved_followups, label);
            }
            den_runtime::runtime_conversations::RuntimeSemanticGroupKind::ToolInteraction
            | den_runtime::runtime_conversations::RuntimeSemanticGroupKind::ArtifactUpdate => {
                push_unique_summary_value(&mut summary.artifact_refs, label);
            }
            den_runtime::runtime_conversations::RuntimeSemanticGroupKind::ApprovalInteraction => {
                push_unique_summary_value(&mut summary.decisions_made, label);
            }
            den_runtime::runtime_conversations::RuntimeSemanticGroupKind::WorkflowUpdate => {
                push_unique_summary_value(&mut summary.workflow_state_refs, label);
            }
            den_runtime::runtime_conversations::RuntimeSemanticGroupKind::PriorCompactionArtifact
            | den_runtime::runtime_conversations::RuntimeSemanticGroupKind::SystemEvent => {
                push_unique_summary_value(&mut summary.important_constraints, label);
            }
        }
        if group.protected {
            push_unique_summary_value(
                &mut summary.important_constraints,
                format!("protected:{:?}", group.kind),
            );
        }
    }
    summary
}

fn push_unique_summary_value(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

pub(super) fn map_canonical_history_page(
    rows: &[PersistedConversationMessage],
    page_limit: u32,
) -> (Vec<AcpConversationHistoryMessage>, bool, Option<String>) {
    let has_more = rows.len() >= page_limit as usize;
    let next_before = rows.last().map(|row| row.sequence_no.to_string());
    let messages = rows
        .iter()
        .rev()
        .filter_map(|row| {
            let role = match (row.message_type.as_str(), row.role.as_deref()) {
                ("user", _) => "user",
                ("assistant", _) => "assistant",
                ("message", Some("user")) => "user",
                ("message", Some("assistant")) => "assistant",
                _ => return None,
            };
            if row.visibility == "diagnostic_only" {
                return None;
            }
            let text = sanitize_visible_transcript_text(&row.content_text);
            if text.trim().is_empty() {
                return None;
            }
            Some(AcpConversationHistoryMessage {
                id: row.provider_message_id.clone().or_else(|| Some(row.sequence_no.to_string())),
                role: role.to_string(),
                text,
                created_at: Some(format_acp_session_timestamp(row.created_at)),
            })
        })
        .collect();
    (messages, has_more, next_before)
}

pub(super) async fn pending_session_title_update_event(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    bear_slug: &str,
    acp_session_id: &str,
) -> Result<Option<AcpGatewayEvent>, CustomError> {
    let Some(session) =
        acp_sessions::find_for_user_bear_session(pool, user_id, bear_slug, acp_session_id).await?
    else {
        return Ok(None);
    };
    if let Some(event) = session_title_update_event_from_row(&session) {
        acp_sessions::mark_title_synced(pool, user_id, bear_id, acp_session_id).await?;
        Ok(Some(event))
    } else {
        Ok(None)
    }
}

pub(super) fn acp_auto_title_instruction(session: &acp_sessions::AcpSessionRow) -> Option<String> {
    let has_title = session
        .conversation_title
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if has_title {
        return None;
    }
    let conversation_id = session.conversation_id.trim();
    let has_conversation_binding = !conversation_id.is_empty()
        && (session
            .resolved_conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            || conversation_id.starts_with("conv-")
            || conversation_id.starts_with("new-"));
    if !has_conversation_binding {
        return None;
    }
    Some(
        "This conversation is currently untitled. Once the main subject is clear enough to summarize in a short, specific title, proactively call `set_conversation_title` in that turn without waiting for the user to ask. Prefer doing this before or alongside your normal response when the topic first becomes clear. Do not title vague openings such as greetings when the subject is not yet clear, and do not automatically rename again after a title has been set unless the human asks for a rename or the existing title is clearly wrong.".to_string(),
    )
}

fn session_title_update_event_from_row(
    session: &acp_sessions::AcpSessionRow,
) -> Option<AcpGatewayEvent> {
    let title = session
        .conversation_title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)?;
    let needs_sync = match (
        session.conversation_title_updated_at,
        session.conversation_title_synced_at,
    ) {
        (Some(updated), Some(synced)) => synced < updated,
        (Some(_), None) => true,
        _ => false,
    };
    needs_sync.then_some(AcpGatewayEvent::SessionInfoUpdate {
        title: Some(title),
        updated_at: session
            .conversation_title_updated_at
            .map(format_acp_session_timestamp),
        meta: None,
    })
}
