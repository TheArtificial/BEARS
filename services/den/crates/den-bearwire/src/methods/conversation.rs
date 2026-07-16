use axum::http::HeaderMap;
use serde_json::{json, Value};
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use bearwire_protocol::{
    methods::ConversationHistoryRequest,
    surface::{SurfaceHistoryEvent, SurfaceResourceRef},
};
use den_http::errors::CustomError;
use den_runtime::bearwire_events;
use den_service::{client_sessions, conversation::persistence, DenState};

use crate::auth::authenticated_bear;
use crate::methods::parse_params;

pub(crate) const FOCUS_TITLE_PREFIX: &str = "⌖ ";

pub(crate) fn project_focus_title(title: Option<String>, focused: bool) -> Option<String> {
    title.map(|title| {
        let bare = title.strip_prefix(FOCUS_TITLE_PREFIX).unwrap_or(&title);
        if focused {
            format!("{FOCUS_TITLE_PREFIX}{bare}")
        } else {
            bare.to_string()
        }
    })
}

pub(crate) async fn conversation_has_active_focus(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    client_session_id: Option<&str>,
) -> Result<bool, den_core::DenError> {
    let focused = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM docket_execution_sessions
            WHERE bear_id = $1
              AND state IN ('active', 'blocked', 'completing', 'paused')
              AND (
                source_conversation_id = $2
                OR ($3::TEXT IS NOT NULL AND source_client_session_id = $3)
                OR ($3::TEXT IS NOT NULL AND session_id = $3)
              )
        )
        "#,
    )
    .bind(bear_id)
    .bind(conversation_id)
    .bind(client_session_id)
    .fetch_one(pool)
    .await?;
    Ok(focused)
}

#[derive(Debug, sqlx::FromRow)]
struct DocketDiagnosticEventRow {
    id: Uuid,
    created_at: OffsetDateTime,
    event_type: String,
    payload: Value,
    job_id: Option<Uuid>,
    job_goal: Option<String>,
    job_status: Option<String>,
    task_id: Option<Uuid>,
    task_title: Option<String>,
}

async fn list_docket_diagnostic_events(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    limit: i64,
) -> Result<Vec<DocketDiagnosticEventRow>, den_core::DenError> {
    sqlx::query_as::<_, DocketDiagnosticEventRow>(
        r#"
        WITH focused_jobs AS (
            SELECT DISTINCT ON (job_id) job_id, task_id
            FROM docket_execution_sessions
            WHERE bear_id = $1
              AND source_conversation_id = $2
            ORDER BY job_id, updated_at DESC
        ), docket_events AS (
            SELECT events.id, events.created_at, events.event_type, events.payload,
                   events.job_id, jobs.goal AS job_goal, jobs.status AS job_status,
                   focused_jobs.task_id AS task_id,
                   focus_tasks.title AS task_title
            FROM bear_job_events events
            JOIN bear_jobs jobs ON jobs.id = events.job_id
            JOIN focused_jobs ON focused_jobs.job_id = events.job_id
            LEFT JOIN bear_tasks focus_tasks ON focus_tasks.id = focused_jobs.task_id
            WHERE events.job_id IN (SELECT job_id FROM focused_jobs)
              AND events.event_type = 'focus_selected'
            UNION ALL
            SELECT events.id, events.created_at, events.event_type, events.payload,
                   tasks.job_id, jobs.goal AS job_goal, jobs.status AS job_status,
                   events.task_id,
                   tasks.title AS task_title
            FROM bear_task_events events
            JOIN bear_tasks tasks ON tasks.id = events.task_id
            JOIN bear_jobs jobs ON jobs.id = tasks.job_id
            WHERE tasks.job_id IN (SELECT job_id FROM focused_jobs)
              AND events.event_type IN ('created', 'updated')
              AND events.payload ? 'definition'
        )
        SELECT id, created_at, event_type, payload, job_id, job_goal, job_status, task_id, task_title
        FROM docket_events
        ORDER BY created_at ASC
        LIMIT $3
        "#,
    )
    .bind(bear_id)
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| den_core::DenError::Database(format!("list docket diagnostic events: {err}")))
}

fn docket_diagnostic_surface_event(row: DocketDiagnosticEventRow) -> Option<Value> {
    match row.event_type.as_str() {
        "focus_selected" => Some(json!(SurfaceHistoryEvent::Message {
            id: Some(format!("docket:{}", row.id)),
            role: "system".to_string(),
            text: format!(
                "Docket focus selected: job={} goal={} status={} task={} state={}",
                row.job_id?,
                row.job_goal.as_deref().unwrap_or("unknown"),
                row.job_status.as_deref().unwrap_or("unknown"),
                row.task_title.as_deref().unwrap_or("unknown task"),
                row.payload
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("active")
            ),
            resources: Vec::<SurfaceResourceRef>::new(),
            created_at: Some(row.created_at.to_string()),
        })),
        "created" | "updated" => {
            let definition = row.payload.get("definition")?;
            let title = definition
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("untitled task");
            Some(json!(SurfaceHistoryEvent::Message {
                id: Some(format!("docket:{}", row.id)),
                role: "system".to_string(),
                text: format!(
                    "Docket task {}: {} ({})",
                    row.event_type,
                    title,
                    row.task_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "unknown task".to_string())
                ),
                resources: Vec::<SurfaceResourceRef>::new(),
                created_at: Some(row.created_at.to_string()),
            }))
        }
        _ => None,
    }
}

fn orientation_diagnostic_text(data: &Value) -> String {
    let kind = data
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let orientation = data.get("orientation").unwrap_or(&Value::Null);
    let focused_job = orientation
        .pointer("/job/job_id")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let task_ref = orientation
        .pointer("/task/task_ref")
        .or_else(|| orientation.pointer("/job/active_task_ref"));
    let task = task_ref
        .and_then(|task| {
            task.get("task_id")
                .or_else(|| task.get("item_id"))
                .and_then(Value::as_str)
        })
        .unwrap_or("none");
    format!(
        "Runtime orientation: kind={} focused_job={} task={}",
        kind, focused_job, task
    )
}

fn trimmed_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

fn surface_resource_refs_from_content_json(content_json: &Value) -> Vec<SurfaceResourceRef> {
    let host_context = content_json
        .get("host_context")
        .or_else(|| content_json.pointer("/prompt_context/host_context"));
    let Some(resources) = host_context
        .and_then(|context| context.get("resources"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    resources
        .iter()
        .filter_map(|resource| {
            let label = trimmed_string(resource, &["label"])
                .or_else(|| trimmed_string(resource, &["name", "title"]))
                .or_else(|| trimmed_string(resource, &["uri", "url"]));
            let uri = trimmed_string(resource, &["uri", "url"]);
            let name = trimmed_string(resource, &["name", "title"]);
            let mime_type = trimmed_string(
                resource,
                &["mime_type", "mimeType", "media_type", "mediaType"],
            );
            if label.is_none() && uri.is_none() && name.is_none() {
                return None;
            }
            Some(SurfaceResourceRef {
                label,
                uri,
                name,
                mime_type,
            })
        })
        .collect()
}

fn surface_text_with_resource_refs(text: String, resources: &[SurfaceResourceRef]) -> String {
    if resources.is_empty() {
        return text;
    }

    let mut rendered = text;
    rendered.push_str("\n\nReferenced resources:");
    for resource in resources {
        let label = resource
            .label
            .as_deref()
            .or(resource.name.as_deref())
            .or(resource.uri.as_deref())
            .unwrap_or("unnamed resource");
        rendered.push_str("\n- ");
        rendered.push_str(label);
        if let Some(uri) = resource.uri.as_deref() {
            if uri != label {
                rendered.push_str(" — ");
                rendered.push_str(uri);
            }
        }
        if let Some(mime_type) = resource.mime_type.as_deref() {
            rendered.push_str(" (");
            rendered.push_str(mime_type);
            rendered.push(')');
        }
    }
    rendered
}

pub(crate) async fn conversation_history_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    conversation_history_like_result(state, headers, params, "conversation_history", "messages")
        .await
}

pub(crate) async fn conversation_surface_history_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    // ponytail: surface replay currently merges structured conversation rows with a small
    // allowlist of persisted BearWire events. The ceiling is pagination/order across independent
    // sequences; replace this with a dedicated persisted surface-event stream when Phase 3 grows.
    conversation_history_like_result(
        state,
        headers,
        params,
        "conversation_surface_history",
        "surface_events",
    )
    .await
}

async fn conversation_history_like_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
    response_kind: &str,
    records_key: &str,
) -> Result<Value, CustomError> {
    let (_user_id, bear) = authenticated_bear(state, headers, params).await?;
    let request: ConversationHistoryRequest = parse_params(params)?;
    let conversation_id = request.conversation_id;
    let before_sequence_no = request.before;
    let limit = request.limit.clamp(1, 100);

    let Some(conversation) =
        persistence::get_conversation_for_external_id(&state.sqlx_pool, bear.id, &conversation_id)
            .await?
    else {
        let mut response = json!({
            "kind": response_kind,
            "conversation_id": conversation_id,
            "has_more": false,
            "next_before": null,
            "missing": true,
        });
        response[records_key] = json!([]);
        return Ok(response);
    };

    let rows = persistence::list_messages_page(
        &state.sqlx_pool,
        conversation.id,
        before_sequence_no,
        limit,
    )
    .await?;
    let has_more = rows.len() >= limit as usize;
    let next_before = rows.last().map(|row| row.sequence_no.to_string());
    let mut messages = rows
        .iter()
        .rev()
        .filter_map(|row| {
            let message = row.to_user_history_record()?;
            let kind = message.kind.as_str();
            match kind {
                "tool_call" => Some(json!(SurfaceHistoryEvent::ToolCall {
                    id: message
                        .message_id
                        .or_else(|| Some(message.sequence_no.to_string())),
                    role: Some(message.role),
                    tool_call_id: message.tool_call_id?,
                    tool_name: message.tool_name?,
                    status: message.status?,
                    arguments: message.arguments,
                    created_at: Some(message.created_at.to_string()),
                })),
                "tool_result" => {
                    let text = (!message.content.trim().is_empty()).then(|| {
                        den_runtime::agent_assist::sanitize_visible_transcript_text(
                            &message.content,
                        )
                    });
                    Some(json!(SurfaceHistoryEvent::ToolResult {
                        id: message
                            .message_id
                            .or_else(|| Some(message.sequence_no.to_string())),
                        role: Some(message.role),
                        tool_call_id: message.tool_call_id?,
                        tool_name: message.tool_name?,
                        status: message.status?,
                        text,
                        raw_output: message.raw_output,
                        created_at: Some(message.created_at.to_string()),
                    }))
                }
                _ => {
                    let mut text = den_runtime::agent_assist::sanitize_visible_transcript_text(
                        &message.content,
                    );
                    if text.trim().is_empty() {
                        return None;
                    }
                    let resources = if records_key == "surface_events" && message.role == "user" {
                        surface_resource_refs_from_content_json(&row.content_json)
                    } else {
                        Vec::new()
                    };
                    text = surface_text_with_resource_refs(text, &resources);
                    Some(json!(SurfaceHistoryEvent::Message {
                        id: message
                            .message_id
                            .or_else(|| Some(message.sequence_no.to_string())),
                        role: message.role,
                        text,
                        resources,
                        created_at: Some(message.created_at.to_string()),
                    }))
                }
            }
        })
        .collect::<Vec<_>>();

    if records_key == "surface_events" {
        if let Some(session) = client_sessions::find_latest_for_bear_conversation(
            &state.sqlx_pool,
            bear.id,
            &conversation_id,
        )
        .await?
        {
            let focused = conversation_has_active_focus(
                &state.sqlx_pool,
                bear.id,
                &conversation_id,
                Some(&session.client_session_id),
            )
            .await?;
            messages.insert(
                0,
                json!(SurfaceHistoryEvent::SessionInfoUpdate {
                    id: Some(format!("session-info:{}", session.client_session_id)),
                    role: Some("system".to_string()),
                    session_id: Some(session.client_session_id.clone()),
                    title: project_focus_title(session.conversation_title.clone(), focused),
                    title_updated_at: session
                        .conversation_title_updated_at
                        .map(|value| value.to_string()),
                    current_mode: Some(session.current_mode.clone()),
                    created_at: Some(
                        session
                            .conversation_title_updated_at
                            .unwrap_or(session.updated_at)
                            .to_string(),
                    ),
                }),
            );

            let surface_event_rows = bearwire_events::list_bearwire_events_after(
                &state.sqlx_pool,
                &session.client_session_id,
                None,
                limit,
            )
            .await?;
            for row in surface_event_rows {
                if row.event_type == "runtime.objective_orientation" {
                    let event_id = row
                        .event
                        .event_id
                        .unwrap_or_else(|| format!("bearwire:{}", row.id));
                    messages.push(json!(SurfaceHistoryEvent::Message {
                        id: Some(event_id),
                        role: "system".to_string(),
                        text: orientation_diagnostic_text(&row.event.data),
                        resources: Vec::<SurfaceResourceRef>::new(),
                        created_at: Some(row.created_at.to_string()),
                    }));
                    continue;
                }

                if row.event_type == "session_info_update" {
                    let title = row
                        .event
                        .data
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|title| !title.is_empty());
                    let updated_at = row.event.data.get("updated_at").and_then(Value::as_str);
                    if title.is_none() && updated_at.is_none() {
                        continue;
                    }
                    let event_id = row
                        .event
                        .event_id
                        .unwrap_or_else(|| format!("bearwire:{}", row.id));
                    messages.push(json!(SurfaceHistoryEvent::SessionInfoUpdate {
                        id: Some(event_id),
                        role: Some("system".to_string()),
                        session_id: Some(session.client_session_id.clone()),
                        title: project_focus_title(title.map(str::to_string), focused),
                        title_updated_at: updated_at.map(str::to_string),
                        current_mode: None,
                        created_at: Some(row.created_at.to_string()),
                    }));
                    continue;
                }

                if row.event_type != "message.reasoning.delta" {
                    continue;
                }
                let delta = row
                    .event
                    .data
                    .get("delta")
                    .or_else(|| row.event.data.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if delta.is_empty() {
                    continue;
                }
                let source = row
                    .event
                    .data
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("provider_reasoning")
                    .to_string();
                let replay_policy = row
                    .event
                    .data
                    .get("replay_policy")
                    .and_then(Value::as_str)
                    .unwrap_or("none")
                    .to_string();
                if replay_policy != "thought" {
                    continue;
                }
                let event_id = row
                    .event
                    .event_id
                    .unwrap_or_else(|| format!("bearwire:{}", row.id));
                messages.push(json!(SurfaceHistoryEvent::ReasoningDelta {
                    id: Some(event_id),
                    role: Some("assistant".to_string()),
                    text: delta.to_string(),
                    source: Some(source),
                    replay_policy: Some(replay_policy),
                    created_at: Some(row.created_at.to_string()),
                }));
            }
        }

        for row in list_docket_diagnostic_events(
            &state.sqlx_pool,
            bear.id,
            &conversation_id,
            i64::from(limit),
        )
        .await?
        {
            if let Some(event) = docket_diagnostic_surface_event(row) {
                messages.push(event);
            }
        }
    }

    if records_key == "surface_events" {
        messages.sort_by(|left, right| {
            let left_created = left
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let right_created = right
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or_default();
            left_created.cmp(right_created)
        });
    }

    let mut response = json!({
        "kind": response_kind,
        "conversation_id": conversation_id,
        "has_more": has_more,
        "next_before": next_before,
    });
    response[records_key] = json!(messages);
    Ok(response)
}
