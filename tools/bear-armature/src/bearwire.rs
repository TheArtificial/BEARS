use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use crate::{
    adapter_contract_context, den_request_context, env_bool, handle_den_event,
    send_agent_message_chunk_for_turn, send_tool_call_update,
    stream_has_successful_terminal_condition, truncate_for_log, AdapterSharedState, AdapterState,
    Config, SseFrameOutcome, SseStreamDiagnostics, ToolCallUpdatePayload,
};

const BEARWIRE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const BEARWIRE_PROMPT_TIMEOUT: Duration = Duration::from_secs(600);
const BEARWIRE_TOOL_RAW_OUTPUT_PREVIEW_CHARS: usize = 24 * 1024;

fn generic_tool_summary(summary: &str) -> bool {
    matches!(summary.trim(), "Tool failed." | "Tool completed.")
}

fn generic_tool_summary_for_tool(summary: &str, tool_name: &str) -> bool {
    let normalized = summary.trim().trim_end_matches('.').to_ascii_lowercase();
    if normalized == format!("finished {}", tool_name.to_ascii_lowercase()) {
        return true;
    }
    if matches!(normalized.as_str(), "tool failed" | "tool completed") {
        return true;
    }
    let status_suffix = normalized
        .strip_suffix(" completed")
        .or_else(|| normalized.strip_suffix(" failed"));
    let Some(prefix) = status_suffix else {
        return false;
    };
    if prefix.strip_prefix("local tool ").is_some() {
        return true;
    }
    let fallback = crate::fallback_tool_title(tool_name).to_ascii_lowercase();
    let friendly = crate::friendly_tool_title(tool_name).to_ascii_lowercase();
    prefix == fallback || prefix == friendly
}

fn default_tool_status_summary(tool_name: &str, failed: bool) -> String {
    if !failed {
        match tool_name {
            "session_info" => return "Inspected session.".to_string(),
            "set_conversation_title" => return "Set conversation title.".to_string(),
            "memory_browse" => return "Browsed memory.".to_string(),
            "memory_read" => return "Read memory.".to_string(),
            "memory_search" => return "Searched memory.".to_string(),
            "memory_write_entry" => return "Wrote memory entry.".to_string(),
            "memory_request_review" => return "Requested memory review.".to_string(),
            "web_fetch" | "local_web_fetch" => return "Fetched URL.".to_string(),
            "web_search" => return "Searched web.".to_string(),
            "list_task_lists" => return "Listed task lists.".to_string(),
            "get_task_list_status" => return "Read task list status.".to_string(),
            "update_task_list" | "update_plan" => return "Updated task list.".to_string(),
            "request_task_list_handoff" | "request_work_handoff" => {
                return "Requested work handoff.".to_string();
            }
            "git_status" => return "Checked git status.".to_string(),
            "git_diff" => return "Read git diff.".to_string(),
            "git_log" => return "Read git log.".to_string(),
            "git_show" => return "Read git revision.".to_string(),
            "git_add" => return "Staged git changes.".to_string(),
            "git_restore" => return "Restored git paths.".to_string(),
            "git_commit" => return "Created git commit.".to_string(),
            "git_stash" => return "Created git stash.".to_string(),
            _ => {}
        }
    }
    let title = crate::friendly_tool_title(tool_name);
    if failed {
        format!("{title} failed.")
    } else {
        format!("{title} completed.")
    }
}

fn command_name_from_tool_event(data: &Value) -> Option<String> {
    let tool_name = data.get("tool_name").and_then(Value::as_str)?;
    if !matches!(
        tool_name,
        "run_command" | "process_run" | "terminal_run_command"
    ) {
        return None;
    }
    let command = data
        .get("args")
        .and_then(|args| args.get("command"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)?;
    let arg_list = data
        .get("args")
        .and_then(|args| args.get("args"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let summary = match command.as_str() {
        "git" | "cargo" => arg_list
            .first()
            .map(|subcommand| format!("{command} {subcommand}"))
            .unwrap_or(command),
        "python" | "python3" if arg_list.first().is_some_and(|arg| arg == "-m") => arg_list
            .get(1)
            .map(|module| format!("{command} -m {module}"))
            .unwrap_or(command),
        _ => command,
    };
    Some(summary)
}

fn default_tool_status_summary_with_context(data: &Value, tool_name: &str, failed: bool) -> String {
    let base = default_tool_status_summary(tool_name, failed);
    let Some(command) = command_name_from_tool_event(data) else {
        return base;
    };
    format!("{base} Command: `{command}`.")
}

fn tool_call_finished_summary(data: &Value, tool_name: &str, failed: bool) -> String {
    let candidate = [
        data.get("error_message").and_then(Value::as_str),
        data.get("summary").and_then(Value::as_str),
        data.get("message").and_then(Value::as_str),
        data.get("content").and_then(Value::as_str),
        data.get("detail").and_then(Value::as_str),
        data.pointer("/diagnostic/message").and_then(Value::as_str),
        data.pointer("/diagnostic/error").and_then(Value::as_str),
        data.pointer("/diagnostic/reason").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|message| {
        !message.is_empty()
            && !generic_tool_summary(message)
            && !generic_tool_summary_for_tool(message, tool_name)
    });

    match candidate {
        Some(message) => message.to_string(),
        None => {
            let structured_preview = data
                .get("structured_content")
                .map(|structured| crate::tool_completion_preview(tool_name, structured))
                .filter(|preview| !preview.trim().is_empty());
            if let Some(preview) = structured_preview {
                return preview;
            }
            if crate::is_placeholder_tool_name(tool_name) {
                let status = if failed { "failed" } else { "completed" };
                let tool_call_id = data
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or("unknown");
                return format!(
                    "Tool call {status} (tool_call_id={tool_call_id}). Details: `{}`",
                    crate::compact_tool_json_detail(data, 1_200)
                );
            }
            default_tool_status_summary_with_context(data, tool_name, failed)
        }
    }
}

fn compact_json_preview(value: &Value, max_chars: usize) -> Value {
    let mut serialized = value.to_string();
    if serialized.chars().count() <= max_chars {
        return value.clone();
    }
    serialized = serialized.chars().take(max_chars).collect::<String>();
    json!({
        "preview": serialized,
        "truncated": true,
        "preview_max_chars": max_chars,
        "original_kind": match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        },
    })
}

fn bearwire_env_value() -> Option<String> {
    std::env::var("BEARS_BEARWIRE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
}

pub(crate) fn enabled() -> bool {
    match bearwire_env_value().as_deref() {
        None | Some("") | Some("auto") => true,
        Some("0" | "false" | "no" | "off" | "disabled") => false,
        Some(_) => env_bool("BEARS_BEARWIRE"),
    }
}

pub(crate) fn required() -> bool {
    true
}

pub(crate) fn mode_summary() -> String {
    let raw = std::env::var("BEARS_BEARWIRE").unwrap_or_else(|_| "<unset>".to_string());
    let mode = if required() {
        "required"
    } else if raw.trim().is_empty() || raw == "<unset>" || raw.trim().eq_ignore_ascii_case("auto") {
        "auto"
    } else if enabled() {
        "enabled"
    } else {
        "disabled"
    };
    format!(
        "{mode} (BEARS_BEARWIRE={raw}, BEARS_BEARWIRE_REQUIRED={})",
        required()
    )
}

pub(crate) async fn protocol_status(http: &reqwest::Client, config: &Config) -> String {
    if !enabled() {
        return format!("disabled; {}", mode_summary());
    }
    match rpc_call(http, config, "initialize", json!({})).await {
        Ok(value) => {
            let protocol = value
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let version = value
                .get("version")
                .and_then(Value::as_i64)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let server_name = value
                .pointer("/server/name")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let server_version = value
                .pointer("/server/version")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let git_sha = value
                .pointer("/server/git_sha")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let legacy = value
                .get("legacy_acp_enabled")
                .and_then(Value::as_bool)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let legacy_deprecated = value
                .get("legacy_acp_deprecated")
                .and_then(Value::as_bool)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let legacy_phase = value
                .get("legacy_acp_removal_phase")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            format!(
                "{}; Den advertises protocol={} version={} server={} {} git_sha={} legacy_acp_enabled={} legacy_acp_deprecated={} legacy_acp_removal_phase={}",
                mode_summary(),
                protocol,
                version,
                server_name,
                server_version,
                git_sha,
                legacy,
                legacy_deprecated,
                legacy_phase,
            )
        }
        Err(err) => format!("{}; initialize failed: {err:#}", mode_summary()),
    }
}

pub(crate) async fn validate_code_token(http: &reqwest::Client, config: &Config) -> Result<()> {
    let initialize = rpc_call(http, config, "initialize", json!({})).await?;
    if initialize.get("protocol").and_then(Value::as_str) != Some("bearwire")
        || initialize.get("version").and_then(Value::as_i64) != Some(1)
    {
        return Err(anyhow!(
            "Den did not advertise BearWire v1 support: {initialize}"
        ));
    }

    let result = rpc_call(
        http,
        config,
        "session.state",
        json!({
            "bear_slug": config.bear,
            "limit": 1,
        }),
    )
    .await?;
    if result.get("kind").and_then(Value::as_str).is_some() {
        Ok(())
    } else {
        Err(anyhow!(
            "BearWire session.state returned unexpected result: {result}"
        ))
    }
}

pub(crate) async fn post_session_open(
    http: &reqwest::Client,
    config: &Config,
    session_id: &str,
    client_context: Value,
    conversation_id: Option<&str>,
    requested_mode: &str,
) -> Result<Value> {
    let cwd = client_context
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    rpc_call(
        http,
        config,
        "session.open",
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "conversation_id": conversation_id,
            "client": config.client,
            "cwd": cwd,
            "mode": requested_mode,
            "adapter_contract": adapter_contract_context(),
            "client_context": client_context,
        }),
    )
    .await
    .context("BearWire session.open failed")
}

pub(crate) async fn handle_prompt(
    http: &reqwest::Client,
    config: &Config,
    adapter_state: &mut AdapterState,
    shared_state: &AdapterSharedState,
    response_id: Value,
    session_id: &str,
    prompt: &str,
    prompt_context: Value,
    client_context: Value,
    conversation_id: Option<&str>,
    requested_mode: &str,
    turn_token: Uuid,
) -> Result<()> {
    let session_result = post_session_open(
        http,
        config,
        session_id,
        client_context.clone(),
        conversation_id,
        requested_mode,
    )
    .await?;

    if crate::bear_debug_verbose() {
        eprintln!(
            "bear-armature: BearWire session.open ok session_id={} result={}",
            session_id,
            truncate_for_log(&session_result.to_string(), 360)
        );
    }
    if let Err(err) = crate::sync_session_model_from_den(
        http,
        Some(config),
        shared_state,
        adapter_state,
        session_id,
    )
    .await
    {
        eprintln!(
            "bear-armature: failed to sync model config after session.open session_id={} error={err:#}",
            session_id
        );
    }

    let cwd = client_context
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let run_result = rpc_call(
        http,
        config,
        "run.start",
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "conversation_id": conversation_id,
            "client": config.client,
            "cwd": cwd,
            "prompt": prompt,
            "prompt_context": prompt_context,
            "requested_mode": requested_mode,
            "adapter_contract": adapter_contract_context(),
            "client_context": client_context,
        }),
    )
    .await
    .context("BearWire run.start failed")?;

    let run_id = run_result
        .get("run_id")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let mut after = run_result
        .get("event_sequence")
        .and_then(Value::as_i64)
        .map(|sequence| sequence.saturating_sub(1));
    if crate::bear_debug_verbose() {
        eprintln!(
            "bear-armature: BearWire run.start accepted session_id={} run_id={} after={:?}",
            session_id, run_id, after
        );
    }

    let mut diagnostics = SseStreamDiagnostics::default();
    let mut saw_done = false;
    let mut saw_visible_output = false;
    let mut saw_tool_activity = false;
    let mut saw_error = false;
    let started = Instant::now();
    let mut last_poll_log = Instant::now();
    let mut logged_initial_wait = false;

    while started.elapsed() < BEARWIRE_PROMPT_TIMEOUT {
        let replay = fetch_events(http, config, session_id, after).await?;
        let replay_count = replay.frames.len();
        let mut max_sequence = after;
        for frame in replay.frames {
            if let Some(sequence) = frame.sequence {
                max_sequence = Some(max_sequence.map_or(sequence, |current| current.max(sequence)));
            }
            let Some(event) = frame.event else {
                continue;
            };
            let outcome = handle_bearwire_event(
                config,
                adapter_state,
                shared_state,
                session_id,
                &event,
                &mut diagnostics,
                turn_token,
            )
            .await?;
            saw_done |= outcome.saw_done;
            saw_visible_output |= outcome.saw_visible_output;
            saw_tool_activity |= outcome.saw_tool_activity;
            saw_error |= outcome.saw_error;
            if saw_done {
                break;
            }
        }
        after = max_sequence;
        if saw_done {
            if crate::bear_debug_verbose() {
                eprintln!(
                    "bear-armature: BearWire run terminal event received session_id={} run_id={} diagnostics={}",
                    session_id,
                    run_id,
                    diagnostics.summary()
                );
            }
            break;
        }
        if !logged_initial_wait
            && started.elapsed() >= Duration::from_secs(5)
            && !saw_visible_output
            && !saw_tool_activity
            && !saw_error
        {
            logged_initial_wait = true;
            if crate::bear_debug_verbose() {
                eprintln!(
                    "bear-armature: BearWire still waiting for first visible/tool event session_id={} run_id={} after={:?} elapsed_ms={} diagnostics={}",
                    session_id,
                    run_id,
                    after,
                    started.elapsed().as_millis(),
                    diagnostics.summary()
                );
            }
        }
        if crate::bear_debug_verbose() && last_poll_log.elapsed() >= Duration::from_secs(5) {
            eprintln!(
                "bear-armature: BearWire polling session_id={} run_id={} after={:?} replay_frames={} elapsed_ms={} diagnostics={}",
                session_id,
                run_id,
                after,
                replay_count,
                started.elapsed().as_millis(),
                diagnostics.summary()
            );
            last_poll_log = Instant::now();
        }
        sleep(BEARWIRE_POLL_INTERVAL).await;
    }

    if !stream_has_successful_terminal_condition(
        saw_visible_output,
        saw_error,
        saw_done,
        saw_tool_activity,
    ) {
        return Err(anyhow!(
            "BEARS BearWire stream completed without visible output, tool activity, or an error. Diagnostics: {}",
            diagnostics.summary()
        ));
    }

    crate::write_prompt_end_turn_response(response_id).await
}

pub(crate) async fn post_session_close(config: &Config, session_id: &str) -> Result<Value> {
    rpc_call(
        &reqwest::Client::new(),
        config,
        "session.close",
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "adapter_contract": adapter_contract_context(),
        }),
    )
    .await
}

pub(crate) async fn post_session_compact(config: &Config, session_id: &str) -> Result<Value> {
    rpc_call(
        &reqwest::Client::new(),
        config,
        "session.compact",
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "adapter_contract": adapter_contract_context(),
        }),
    )
    .await
}

pub(crate) async fn post_run_cancel(config: &Config, session_id: &str) -> Result<Value> {
    rpc_call(
        &reqwest::Client::new(),
        config,
        "run.cancel",
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "adapter_contract": adapter_contract_context(),
        }),
    )
    .await
}

pub(crate) async fn post_resource_update(
    config: &Config,
    session_id: &str,
    resource: Value,
) -> Result<Value> {
    rpc_call(
        &reqwest::Client::new(),
        config,
        "resource.update",
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "resource": resource,
            "adapter_contract": adapter_contract_context(),
        }),
    )
    .await
}

pub(crate) fn tool_result_rpc_params(
    config: &Config,
    session_id: &str,
    run_id: &str,
    tool_call_id: &str,
    payload: &Value,
) -> Value {
    let status = payload
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("ok");
    let error = if status == "ok" {
        Value::Null
    } else {
        payload
            .get("error")
            .cloned()
            .unwrap_or_else(|| payload.get("diagnostic").cloned().unwrap_or(Value::Null))
    };
    json!({
        "bear_slug": config.bear,
        "session_id": session_id,
        "run_id": run_id,
        "tool_call_id": tool_call_id,
        "tool_name": payload.get("tool_name").cloned().unwrap_or(Value::Null),
        "status": status,
        "content": payload.get("content").cloned().unwrap_or(Value::Null),
        "structured_content": payload.get("structured_content").cloned().unwrap_or(Value::Null),
        "diagnostic": payload.get("diagnostic").cloned().unwrap_or(Value::Null),
        "error": error,
        "adapter_contract": adapter_contract_context(),
    })
}

pub(crate) async fn post_tool_result(
    config: &Config,
    session_id: &str,
    run_id: &str,
    tool_call_id: &str,
    payload: Value,
) -> Result<Value> {
    rpc_call(
        &reqwest::Client::new(),
        config,
        "client.tool.result",
        tool_result_rpc_params(config, session_id, run_id, tool_call_id, &payload),
    )
    .await
}

pub(crate) async fn post_permission_result(
    config: &Config,
    session_id: &str,
    run_id: &str,
    permission_id: &str,
    payload: Value,
) -> Result<Value> {
    rpc_call(
        &reqwest::Client::new(),
        config,
        "client.permission.result",
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "run_id": run_id,
            "permission_id": permission_id,
            "obligation_id": payload.get("obligation_id").cloned().unwrap_or(Value::Null),
            "decision": payload.get("decision").and_then(Value::as_str).unwrap_or("denied"),
            "reason": payload.get("reason").cloned().unwrap_or(Value::Null),
            "adapter_contract": adapter_contract_context(),
        }),
    )
    .await
}

pub(crate) async fn try_handle_prompt(
    http: &reqwest::Client,
    config: &Config,
    adapter_state: &mut AdapterState,
    shared_state: &AdapterSharedState,
    response_id: Value,
    session_id: &str,
    prompt: &str,
    prompt_context: Value,
    client_context: Value,
    conversation_id: Option<&str>,
    requested_mode: &str,
    turn_token: Uuid,
) -> Result<bool> {
    if !enabled() {
        return Err(anyhow!(
            "BearWire is disabled in this adapter process, and legacy ACP HTTP is retired. Enable BearWire by setting BEARS_BEARWIRE=auto or true."
        ));
    }
    match handle_prompt(
        http,
        config,
        adapter_state,
        shared_state,
        response_id,
        session_id,
        prompt,
        prompt_context,
        client_context,
        conversation_id,
        requested_mode,
        turn_token,
    )
    .await
    {
        Ok(()) => Ok(true),
        Err(err) => Err(err),
    }
}

pub(crate) async fn rpc_call(
    http: &reqwest::Client,
    config: &Config,
    method: &str,
    params: Value,
) -> Result<Value> {
    let url = format!("{}/bearwire/v1/rpc", config.api_url);
    let response = http
        .post(&url)
        .header(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", config.token))?,
        )
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": format!("bearwire-{}", Uuid::new_v4().simple()),
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .with_context(|| den_request_context(&url))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "BearWire RPC {method} HTTP {status}: {}",
            body.trim()
        ));
    }
    let value: Value = serde_json::from_str(&body)
        .with_context(|| format!("parse BearWire RPC {method} JSON: {body}"))?;
    if let Some(error) = value.get("error") {
        return Err(anyhow!("BearWire RPC {method} error: {error}"));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

struct BearWireReplay {
    frames: Vec<BearWireFrame>,
}

struct BearWireFrame {
    sequence: Option<i64>,
    event: Option<Value>,
}

async fn fetch_events(
    http: &reqwest::Client,
    config: &Config,
    session_id: &str,
    after: Option<i64>,
) -> Result<BearWireReplay> {
    let mut url = format!(
        "{}/bearwire/v1/sessions/{}/events?bear_slug={}",
        config.api_url,
        urlencoding::encode(session_id),
        urlencoding::encode(&config.bear)
    );
    if let Some(after) = after {
        url.push_str("&after=");
        url.push_str(&after.to_string());
    }
    let response = http
        .get(&url)
        .header(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", config.token))?,
        )
        .send()
        .await
        .with_context(|| den_request_context(&url))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("BearWire events HTTP {status}: {}", body.trim()));
    }

    let mut buffer = Vec::<u8>::new();
    let mut frames = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        buffer.extend_from_slice(&chunk.context("read BearWire events chunk")?);
        while let Some(pos) = buffer.windows(2).position(|window| window == b"\n\n") {
            let frame: Vec<u8> = buffer.drain(..pos + 2).collect();
            frames.push(parse_event_frame(&frame)?);
        }
    }
    if !buffer.is_empty() {
        frames.push(parse_event_frame(&buffer)?);
    }
    Ok(BearWireReplay { frames })
}

fn bearwire_tool_event_to_legacy_tool_request(event: &Value, approval_required: bool) -> Value {
    let data = event.get("data").unwrap_or(&Value::Null);
    let tool_call = data.get("tool_call").unwrap_or(&Value::Null);
    let tool_call_id = tool_call
        .get("id")
        .or_else(|| data.get("tool_call_id"))
        .and_then(Value::as_str)
        .or_else(|| resource_ref_id(event, "tool_call"))
        .unwrap_or("unknown");
    let tool_name = tool_call
        .get("name")
        .or_else(|| data.get("tool_name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let approval_request_id = data
        .get("approval_request_id")
        .and_then(Value::as_str)
        .or_else(|| resource_ref_id(event, "permission_request"));
    let mut legacy = json!({
        "type": "tool_request",
        "run_id": event.get("run_id").and_then(Value::as_str),
        "tool_call_id": tool_call_id,
        "tool_name": tool_name,
        "title": tool_call
            .get("title")
            .or_else(|| data.get("title"))
            .cloned()
            .unwrap_or(Value::Null),
        "args": tool_call
            .get("arguments")
            .or_else(|| data.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({})),
        "approval_request_id": approval_request_id,
        "approval": {
            "required": approval_required,
            "reason": data.get("reason").cloned().unwrap_or(Value::Null),
        },
    });
    if let Some(display) = tool_call.get("display").or_else(|| data.get("display")) {
        legacy["display"] = display.clone();
    }
    legacy
}

fn bearwire_session_info_update_to_legacy(event: &Value) -> Value {
    let data = event.get("data").unwrap_or(&Value::Null);
    json!({
        "type": "session_info_update",
        "title": event
            .get("title")
            .or_else(|| data.get("title"))
            .cloned()
            .unwrap_or(Value::Null),
        "updated_at": event
            .get("updated_at")
            .or_else(|| data.get("updated_at"))
            .cloned()
            .unwrap_or(Value::Null),
        "meta": event
            .get("meta")
            .or_else(|| data.get("meta"))
            .cloned()
            .unwrap_or(Value::Null),
    })
}

fn bearwire_permission_event_to_legacy_permission_request(event: &Value) -> Value {
    let data = event.get("data").unwrap_or(&Value::Null);
    let tool_call = data.get("tool_call").unwrap_or(&Value::Null);
    let permission = data.get("permission").unwrap_or(&Value::Null);
    let permission_id = permission
        .get("id")
        .or_else(|| data.get("approval_request_id"))
        .or_else(|| data.get("permission_id"))
        .and_then(Value::as_str)
        .or_else(|| resource_ref_id(event, "permission_request"))
        .unwrap_or("unknown");
    let tool_call_id = tool_call
        .get("id")
        .or_else(|| data.get("tool_call_id"))
        .and_then(Value::as_str)
        .or_else(|| resource_ref_id(event, "tool_call"))
        .unwrap_or(permission_id);
    let tool_name = tool_call
        .get("name")
        .or_else(|| data.get("tool_name"))
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let mut legacy = json!({
        "type": "permission_request",
        "run_id": event.get("run_id").and_then(Value::as_str),
        "obligation_id": data.get("obligation_id").cloned().unwrap_or(Value::Null),
        "expected_client_method": data.get("expected_client_method").cloned().unwrap_or(Value::Null),
        "permission_id": permission_id,
        "tool_call_id": tool_call_id,
        "tool_name": tool_name,
        "title": tool_call
            .get("title")
            .or_else(|| permission.get("title"))
            .or_else(|| data.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("Permission request"),
        "reason": permission
            .get("reason")
            .or_else(|| data.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("BEARS requests permission."),
        "target": tool_call
            .get("arguments")
            .or_else(|| data.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({ "kind": "tool_call" })),
    });
    if let Some(display) = tool_call.get("display").or_else(|| data.get("display")) {
        legacy["display"] = display.clone();
    }
    legacy
}

fn bearwire_plan_update_entries(event: &Value) -> Value {
    let data = event.get("data").unwrap_or(&Value::Null);
    data.pointer("/detail/entries")
        .or_else(|| data.pointer("/detail/plan/items"))
        .or_else(|| data.pointer("/plan/items"))
        .or_else(|| data.get("entries"))
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn bearwire_run_failed_user_message(event: &Value) -> String {
    let data = event.get("data").unwrap_or(&Value::Null);
    let user_message = data
        .get("user_message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty());
    let message = data
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .unwrap_or("BearWire run failed");
    let reason = data
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|reason| !reason.is_empty());
    let error_type = data
        .get("error_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|error_type| !error_type.is_empty());
    let detail = data
        .get("detail")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|detail| !detail.is_empty() && *detail != message);
    let run_id = event
        .get("run_id")
        .and_then(Value::as_str)
        .or_else(|| data.get("run_id").and_then(Value::as_str));
    let mut rendered = if let Some(user_message) = user_message {
        format!("BEARS prompt stopped: {user_message}")
    } else if let Some(reason) = reason {
        format!("BEARS run failed ({reason}): {message}")
    } else {
        format!("BEARS run failed: {message}")
    };
    if user_message.is_none() {
        if let Some(error_type) = error_type {
            rendered.push_str(&format!("\n\nError type: `{error_type}`"));
        }
        if let Some(detail) = detail {
            rendered.push_str(&format!("\n\nDetail: {}", truncate_for_log(detail, 1200)));
        }
    }
    if let Some(run_id) = run_id {
        rendered.push_str(&format!("\n\nRun: `{run_id}`"));
    }
    rendered
}

fn bearwire_run_failed_stderr_context(event: &Value) -> Option<String> {
    let data = event.get("data")?;
    let context = data.get("context")?;
    if context.is_null() {
        return None;
    }
    Some(truncate_for_log(&context.to_string(), 1200))
}

async fn handle_bearwire_tool_call_finished_event(
    shared_state: &AdapterSharedState,
    session_id: &str,
    event: &Value,
    failed: bool,
) -> Result<()> {
    let data = event.get("data").unwrap_or(&Value::Null);
    let lookup_tool_call_id = data
        .pointer("/tool_call/id")
        .and_then(Value::as_str)
        .or_else(|| data.get("tool_call_id").and_then(Value::as_str))
        .or_else(|| resource_ref_id(event, "tool_call"))
        .unwrap_or("unknown");
    let cached = shared_state
        .tool_tasks
        .get(session_id, lookup_tool_call_id)
        .await;
    let (tool_call_id, tool_name) = bearwire_finished_tool_identity(
        event,
        cached.as_ref().map(|record| record.tool_name.as_str()),
    );
    let summary = tool_call_finished_summary(data, &tool_name, failed);
    let status = if failed { "failed" } else { "completed" };
    let mut legacy = json!({
        "type": "tool_result",
        "run_id": event.get("run_id").and_then(Value::as_str),
        "tool_call_id": tool_call_id,
        "tool_name": tool_name.clone(),
    });
    if let Some(args) = cached
        .and_then(|record| record.input_args)
        .or_else(|| data.pointer("/tool_call/arguments").cloned())
        .or_else(|| data.get("arguments").cloned())
    {
        legacy["args"] = args;
    }
    if let Some(display) = data
        .pointer("/tool_call/display")
        .or_else(|| data.get("display"))
    {
        legacy["display"] = display.clone();
    }
    send_tool_call_update(
        session_id,
        &tool_call_id,
        &tool_name,
        ToolCallUpdatePayload {
            status,
            text: &summary,
            event: Some(&legacy),
            raw_output: Some(compact_json_preview(
                data,
                BEARWIRE_TOOL_RAW_OUTPUT_PREVIEW_CHARS,
            )),
            extra_content: Vec::new(),
        },
    )
    .await
}

fn bearwire_finished_tool_identity(
    event: &Value,
    cached_tool_name: Option<&str>,
) -> (String, String) {
    let data = event.get("data").unwrap_or(&Value::Null);
    let tool_call = data.get("tool_call").unwrap_or(&Value::Null);
    let tool_call_id = tool_call
        .get("id")
        .or_else(|| data.get("tool_call_id"))
        .and_then(Value::as_str)
        .or_else(|| resource_ref_id(event, "tool_call"))
        .unwrap_or("unknown")
        .to_string();
    let tool_name = tool_call
        .get("name")
        .or_else(|| data.get("tool_name"))
        .and_then(Value::as_str)
        .filter(|name| !crate::is_placeholder_tool_name(name))
        .map(str::to_string)
        .or_else(|| cached_tool_name.map(str::to_string))
        .unwrap_or_else(|| "tool".to_string());
    (tool_call_id, tool_name)
}

fn resource_ref_id<'a>(event: &'a Value, kind: &str) -> Option<&'a str> {
    event
        .get("resource_refs")
        .and_then(Value::as_array)?
        .iter()
        .find(|resource| resource.get("kind").and_then(Value::as_str) == Some(kind))?
        .get("id")
        .and_then(Value::as_str)
}

fn parse_event_frame(frame: &[u8]) -> Result<BearWireFrame> {
    let text = String::from_utf8_lossy(frame);
    let mut sequence = None;
    let mut event = None;
    for line in text.lines() {
        if let Some(id) = line.strip_prefix("id:") {
            sequence = id.trim().parse::<i64>().ok();
            continue;
        }
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let notification: Value = serde_json::from_str(data).context("parse BearWire SSE data")?;
        event = notification.get("params").cloned().or(Some(notification));
    }
    Ok(BearWireFrame { sequence, event })
}

async fn handle_bearwire_event(
    config: &Config,
    adapter_state: &mut AdapterState,
    shared_state: &AdapterSharedState,
    session_id: &str,
    event: &Value,
    diagnostics: &mut SseStreamDiagnostics,
    turn_token: Uuid,
) -> Result<SseFrameOutcome> {
    diagnostics.frames += 1;
    let mut outcome = SseFrameOutcome::default();
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    diagnostics.observe_event(event);

    match ty {
        "message.delta" => {
            let text = event
                .get("data")
                .and_then(|data| data.get("delta").or_else(|| data.get("text")))
                .and_then(Value::as_str)
                .unwrap_or("");
            outcome.saw_visible_output = !text.is_empty();
            if !text.is_empty() {
                let legacy = json!({ "type": "assistant_text_delta", "text": text });
                handle_den_event(
                    config,
                    adapter_state,
                    shared_state,
                    session_id,
                    &legacy,
                    turn_token,
                )
                .await?;
            }
        }
        "message.reasoning.delta" => {
            let text = event
                .get("data")
                .and_then(|data| data.get("delta").or_else(|| data.get("text")))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !text.is_empty() {
                let legacy = json!({ "type": "reasoning_text_delta", "text": text });
                handle_den_event(
                    config,
                    adapter_state,
                    shared_state,
                    session_id,
                    &legacy,
                    turn_token,
                )
                .await?;
            }
        }
        "run.progress" => {
            let data = event.get("data").unwrap_or(&Value::Null);
            let text = data.get("text").and_then(Value::as_str).unwrap_or("");
            let kind = data
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("progress");
            if kind == "plan_update" {
                let entries = bearwire_plan_update_entries(event);
                let legacy = json!({ "type": "plan_update", "entries": entries });
                outcome.saw_visible_output = true;
                outcome.saw_tool_activity = true;
                diagnostics.saw_tool_activity = true;
                handle_den_event(
                    config,
                    adapter_state,
                    shared_state,
                    session_id,
                    &legacy,
                    turn_token,
                )
                .await?;
                return Ok(outcome);
            }
            let elapsed_ms = data.get("elapsed_ms").and_then(Value::as_u64);
            // Progress is observability, not model-visible output. Do not let it satisfy
            // prompt completion checks or suppress first-assistant/tool-event diagnostics.
            if crate::bear_debug_verbose() {
                eprintln!(
                    "bear-armature: BearWire progress session_id={} run_id={} kind={} elapsed_ms={} text={}",
                    session_id,
                    event
                        .get("run_id")
                        .and_then(Value::as_str)
                        .unwrap_or("<unknown>"),
                    kind,
                    elapsed_ms
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    if text.is_empty() { "<empty>" } else { text }
                );
            }
            if !text.is_empty() {
                let legacy = json!({ "type": "status_text", "text": text });
                handle_den_event(
                    config,
                    adapter_state,
                    shared_state,
                    session_id,
                    &legacy,
                    turn_token,
                )
                .await?;
            }
        }
        "session_info_update" => {
            let legacy = bearwire_session_info_update_to_legacy(event);
            handle_den_event(
                config,
                adapter_state,
                shared_state,
                session_id,
                &legacy,
                turn_token,
            )
            .await?;
        }
        "session.bound" => {
            let conversation_id = event
                .pointer("/data/binding/conversation_id")
                .or_else(|| event.pointer("/data/conversation_id"))
                .and_then(Value::as_str);
            if let Some(conversation_id) = conversation_id {
                let legacy = json!({
                    "type": "conversation_resolved",
                    "conversation_id": conversation_id,
                });
                handle_den_event(
                    config,
                    adapter_state,
                    shared_state,
                    session_id,
                    &legacy,
                    turn_token,
                )
                .await?;
            }
        }
        "run.completed" => {
            outcome.saw_done = true;
            let legacy = json!({ "type": "turn_complete", "outcome": "ok" });
            handle_den_event(
                config,
                adapter_state,
                shared_state,
                session_id,
                &legacy,
                turn_token,
            )
            .await?;
        }
        "run.failed" => {
            let message = bearwire_run_failed_user_message(event);
            eprintln!(
                "bear-armature: BearWire run failed session_id={} message={}",
                session_id,
                truncate_for_log(&message, 500)
            );
            if let Some(context) = bearwire_run_failed_stderr_context(event) {
                eprintln!(
                    "bear-armature: BearWire run failed diagnostic session_id={} context={}",
                    session_id, context
                );
            }
            return Err(anyhow!(message));
        }
        "run.cancelled" => {
            outcome.saw_done = true;
            outcome.saw_error = true;
            outcome.saw_visible_output = true;
            diagnostics.saw_error = true;
            diagnostics.saw_visible_output = true;
            let message = "BEARS request was cancelled.";
            eprintln!(
                "bear-armature: BearWire run cancelled session_id={} run_id={}",
                session_id,
                event
                    .get("run_id")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
            );
            send_agent_message_chunk_for_turn(shared_state, session_id, turn_token, message)
                .await?;
        }
        "tool_call.requested" => {
            outcome.saw_tool_activity = true;
            diagnostics.saw_tool_activity = true;
            let legacy = bearwire_tool_event_to_legacy_tool_request(event, false);
            handle_den_event(
                config,
                adapter_state,
                shared_state,
                session_id,
                &legacy,
                turn_token,
            )
            .await?;
        }
        "tool_call.completed" | "tool_call.warning" | "tool_call.cancelled" => {
            outcome.saw_tool_activity = true;
            diagnostics.saw_tool_activity = true;
            handle_bearwire_tool_call_finished_event(shared_state, session_id, event, false)
                .await?;
        }
        "tool_call.failed" => {
            outcome.saw_tool_activity = true;
            outcome.saw_error = true;
            diagnostics.saw_tool_activity = true;
            diagnostics.saw_error = true;
            handle_bearwire_tool_call_finished_event(shared_state, session_id, event, true).await?;
        }
        "client.waiting" => {
            outcome.saw_tool_activity = true;
            outcome.saw_visible_output = true;
            diagnostics.saw_tool_activity = true;
            diagnostics.saw_visible_output = true;
            let data = event.get("data").unwrap_or(&Value::Null);
            let expected = data
                .get("expected_client_method")
                .and_then(Value::as_str)
                .unwrap_or("");
            let obligation_id = data
                .get("obligation_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty());
            let permission_id = data
                .pointer("/permission/id")
                .and_then(Value::as_str)
                .or_else(|| data.get("approval_request_id").and_then(Value::as_str))
                .or_else(|| data.get("permission_id").and_then(Value::as_str))
                .or_else(|| resource_ref_id(event, "permission_request"))
                .map(str::trim)
                .filter(|id| !id.is_empty());
            if obligation_id.is_none()
                || permission_id.is_none()
                || expected != "client.permission.result"
            {
                return Err(anyhow!(
                    "BearWire client.waiting missing answerable permission obligation for session {session_id}; refusing to show an unanswerable permission prompt"
                ));
            }
            let legacy = bearwire_permission_event_to_legacy_permission_request(event);
            handle_den_event(
                config,
                adapter_state,
                shared_state,
                session_id,
                &legacy,
                turn_token,
            )
            .await?;
        }
        "tool_call.blocked" => {
            if crate::bear_debug_verbose() {
                eprintln!(
                    "bear-armature: ignoring legacy BearWire tool_call.blocked session_id={}; actionable permission waits must use client.waiting",
                    session_id
                );
            }
        }
        "permission.granted" | "permission.denied" | "permission.expired" => {
            diagnostics.saw_tool_activity = true;
            outcome.saw_tool_activity = true;
            if crate::bear_debug_verbose() {
                eprintln!(
                    "bear-armature: BearWire permission event session_id={} type={} permission_id={}",
                    session_id,
                    ty,
                    event
                        .pointer("/data/permission_id")
                        .and_then(Value::as_str)
                        .unwrap_or("<unknown>")
                );
            }
        }
        "run.paused" => {
            // `run.paused` is status/diagnostic state only. Actionable waits must arrive
            // as `client.waiting` with a persisted obligation. Keep this out of normal
            // stderr so stale pause status cannot look like a fresh permission request
            // after the armature already answered the matching obligation.
            if crate::bear_debug_verbose() {
                let reason = event
                    .pointer("/data/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("paused");
                let resume_token = event
                    .pointer("/data/resume_token")
                    .and_then(Value::as_str)
                    .unwrap_or("<none>");
                eprintln!(
                    "bear-armature: BearWire run paused status-only session_id={} reason={} resume_token={}",
                    session_id, reason, resume_token
                );
            }
        }
        "permission.requested" => {
            if crate::bear_debug_verbose() {
                eprintln!(
                    "bear-armature: ignoring legacy BearWire permission.requested session_id={}; actionable permission waits must use client.waiting",
                    session_id
                );
            }
        }
        "session.opened" | "session.state" | "run.accepted" | "run.started" => {}
        _ => {
            diagnostics.observe_unknown(event);
            eprintln!(
                "bear-armature: unknown BearWire event type {:?}; sample={}",
                ty,
                truncate_for_log(&event.to_string(), 240)
            );
        }
    }

    diagnostics.saw_visible_output |= outcome.saw_visible_output;
    diagnostics.saw_tool_activity |= outcome.saw_tool_activity;
    diagnostics.saw_error |= outcome.saw_error;
    diagnostics.saw_turn_complete |= outcome.saw_done;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bearwire_requested_tool_projects_to_executable_tool_request() {
        let event = json!({
            "type": "tool_call.requested",
            "run_id": "run-1",
            "data": {
                "tool_call_id": "call-1",
                "tool_name": "fs_read_text_file",
                "arguments": { "path": "/workspace/README.md" }
            }
        });

        let legacy = bearwire_tool_event_to_legacy_tool_request(&event, false);

        assert_eq!(legacy["type"], "tool_request");
        assert_eq!(legacy["tool_name"], "fs_read_text_file");
        assert_eq!(legacy["args"]["path"], "/workspace/README.md");
        assert_eq!(legacy["approval"]["required"], false);
    }

    #[test]
    fn bearwire_requested_tool_prefers_canonical_tool_call_shape() {
        let event = json!({
            "type": "tool_call.requested",
            "run_id": "run-1",
            "data": {
                "tool_call_id": "legacy-call",
                "tool_name": "legacy_tool",
                "arguments": { "path": "/legacy" },
                "display": { "title": "Legacy title" },
                "tool_call": {
                    "id": "call-1",
                    "name": "fs_read_text_file",
                    "title": "Read file",
                    "kind": "function",
                    "arguments": { "path": "/workspace/README.md" },
                    "display": {
                        "title": "Read /workspace/README.md",
                        "progress": "Reading file"
                    }
                }
            }
        });

        let legacy = bearwire_tool_event_to_legacy_tool_request(&event, false);

        assert_eq!(legacy["type"], "tool_request");
        assert_eq!(legacy["tool_call_id"], "call-1");
        assert_eq!(legacy["tool_name"], "fs_read_text_file");
        assert_eq!(legacy["title"], "Read file");
        assert_eq!(legacy["args"]["path"], "/workspace/README.md");
        assert_eq!(legacy["display"]["title"], "Read /workspace/README.md");
        assert_eq!(legacy["display"]["progress"], "Reading file");
    }

    #[test]
    fn bearwire_finished_tool_prefers_canonical_tool_call_identity() {
        let event = json!({
            "type": "tool_call.completed",
            "run_id": "run-1",
            "resource_refs": [
                { "kind": "tool_call", "id": "resource-call" }
            ],
            "data": {
                "tool_call_id": "legacy-call",
                "tool_name": "legacy_tool",
                "tool_call": {
                    "id": "call-1",
                    "name": "fs_read_text_file",
                    "title": "Read file",
                    "arguments": { "path": "/workspace/README.md" }
                },
                "summary": "Read file."
            }
        });

        let (tool_call_id, tool_name) = bearwire_finished_tool_identity(&event, Some("cached_tool"));

        assert_eq!(tool_call_id, "call-1");
        assert_eq!(tool_name, "fs_read_text_file");
    }

    #[test]
    fn bearwire_session_info_update_projects_nested_title_to_legacy_event() {
        let event = json!({
            "type": "session_info_update",
            "data": {
                "title": "Visible conversation title",
                "updated_at": "2026-07-05T19:00:00Z"
            }
        });

        let legacy = bearwire_session_info_update_to_legacy(&event);

        assert_eq!(legacy["type"], "session_info_update");
        assert_eq!(legacy["title"], "Visible conversation title");
        assert_eq!(legacy["updated_at"], "2026-07-05T19:00:00Z");
    }

    #[test]
    fn bearwire_client_waiting_projects_to_permission_request() {
        let event = json!({
            "type": "client.waiting",
            "run_id": "run-web-1",
            "resource_refs": [
                { "kind": "client_obligation", "id": "obl-web-1" },
                { "kind": "tool_call", "id": "call-web-1" },
                { "kind": "permission_request", "id": "perm-web-1" }
            ],
            "data": {
                "obligation_id": "obl-web-1",
                "expected_client_method": "client.permission.result",
                "tool_call": {
                    "id": "call-web-1",
                    "name": "web_fetch",
                    "title": "Fetch URL",
                    "kind": "function",
                    "arguments": { "kind": "url", "url": "https://example.com/", "host": "example.com" },
                    "display": {
                        "label": "Fetch URL",
                        "progress": "Waiting for permission",
                        "category": "network"
                    }
                },
                "permission": {
                    "id": "perm-web-1",
                    "reason": "BEARS wants to fetch https://example.com/."
                }
            }
        });

        let legacy = bearwire_permission_event_to_legacy_permission_request(&event);

        assert_eq!(legacy["type"], "permission_request");
        assert_eq!(legacy["run_id"], "run-web-1");
        assert_eq!(legacy["obligation_id"], "obl-web-1");
        assert_eq!(legacy["expected_client_method"], "client.permission.result");
        assert_eq!(legacy["permission_id"], "perm-web-1");
        assert_eq!(legacy["tool_call_id"], "call-web-1");
        assert_eq!(legacy["tool_name"], "web_fetch");
        assert_eq!(legacy["target"]["url"], "https://example.com/");
        assert_eq!(legacy["display"]["progress"], "Waiting for permission");
        assert_eq!(legacy["display"]["category"], "network");
    }

    #[test]
    fn bearwire_blocked_tool_projects_to_permission_request() {
        let event = json!({
            "type": "tool_call.blocked",
            "run_id": "run-web-1",
            "resource_refs": [
                { "kind": "tool_call", "id": "call-web-1" },
                { "kind": "permission_request", "id": "perm-web-1" }
            ],
            "data": {
                "tool_call_id": "call-web-1",
                "tool_name": "web_fetch",
                "title": "Fetch URL",
                "reason": "BEARS wants to fetch https://example.com/.",
                "arguments": { "kind": "url", "url": "https://example.com/", "host": "example.com" }
            }
        });

        let legacy = bearwire_permission_event_to_legacy_permission_request(&event);

        assert_eq!(legacy["type"], "permission_request");
        assert_eq!(legacy["run_id"], "run-web-1");
        assert_eq!(legacy["permission_id"], "perm-web-1");
        assert_eq!(legacy["tool_call_id"], "call-web-1");
        assert_eq!(legacy["tool_name"], "web_fetch");
        assert_eq!(legacy["target"]["url"], "https://example.com/");
    }

    #[test]
    fn bearwire_run_failed_user_message_includes_reason_and_run_id() {
        let event = json!({
            "type": "run.failed",
            "run_id": "run-123",
            "data": {
                "reason": "stream_error",
                "message": "Runtime stopped before producing assistant output: max_steps_exceeded"
            }
        });

        let message = bearwire_run_failed_user_message(&event);

        assert!(message.contains("stream_error"));
        assert!(message.contains("max_steps_exceeded"));
        assert!(message.contains("run-123"));
    }

    #[test]
    fn compact_json_preview_truncates_large_raw_output() {
        let value = json!({ "content": "x".repeat(40 * 1024), "status": "ok" });

        let preview = compact_json_preview(&value, 1024);

        assert_eq!(preview["truncated"], true);
        assert_eq!(preview["preview_max_chars"], 1024);
        assert_eq!(preview["original_kind"], "object");
        assert!(preview["preview"].as_str().unwrap().chars().count() <= 1024);
    }

    #[test]
    fn bearwire_run_failed_user_message_includes_error_detail() {
        let event = json!({
            "type": "run.failed",
            "run_id": "run-err",
            "data": {
                "message": "LLM responses provider error",
                "error_type": "invalid_request_error",
                "detail": "{\"code\":\"unsupported_parameter\",\"message\":\"tools[64].name is invalid\"}"
            }
        });

        let message = bearwire_run_failed_user_message(&event);

        assert!(message.contains("LLM responses provider error"));
        assert!(message.contains("invalid_request_error"));
        assert!(message.contains("unsupported_parameter"));
        assert!(message.contains("tools[64].name is invalid"));
        assert!(message.contains("run-err"));
    }

    #[test]
    fn bearwire_run_failed_user_message_has_fallback() {
        let event = json!({ "type": "run.failed" });

        let message = bearwire_run_failed_user_message(&event);

        assert_eq!(message, "BEARS run failed: BearWire run failed");
    }

    #[test]
    fn bearwire_run_failed_user_message_prefers_friendly_user_message() {
        let event = json!({
            "type": "run.failed",
            "run_id": "run-budget",
            "data": {
                "reason": "runtime_internal",
                "message": "I stopped because this turn exhausted its wall-clock budget (elapsed=252985ms/limit=240000ms).",
                "user_message": "BEARS stopped this turn after it ran too long. Recent tool results were preserved, but no final answer was delivered. Start a fresh turn to continue safely.",
                "context": {
                    "diagnostic": {
                        "elapsed_ms": 252985,
                        "limit_ms": 240000
                    }
                }
            }
        });

        let message = bearwire_run_failed_user_message(&event);
        let context = bearwire_run_failed_stderr_context(&event).expect("stderr context");

        assert!(message.starts_with("BEARS prompt stopped:"), "{message}");
        assert!(message.contains("ran too long"), "{message}");
        assert!(!message.contains("runtime_internal"), "{message}");
        assert!(!message.contains("elapsed=252985"), "{message}");
        assert!(context.contains("252985"), "{context}");
    }

    #[test]
    fn bearwire_run_failed_user_message_omits_context_but_stderr_helper_keeps_it() {
        let event = json!({
            "type": "run.failed",
            "run_id": "run-timeout",
            "data": {
                "reason": "continuation_watchdog_timeout",
                "message": "Den received the client result and started continuation request req-123, but no runtime event arrived within 30000ms.",
                "context": {
                    "continuation_request_id": "req-123",
                    "watchdog_timeout_ms": 30000,
                    "runtime_event_count": 0
                }
            }
        });

        let message = bearwire_run_failed_user_message(&event);
        let context = bearwire_run_failed_stderr_context(&event).expect("stderr context");

        assert!(!message.contains("Context:"));
        assert!(message.contains("run-timeout"));
        assert!(context.contains("continuation_request_id"));
        assert!(context.contains("req-123"));
    }

    #[test]
    fn tool_call_finished_summary_uses_tool_name_when_upstream_summary_is_generic() {
        let data = json!({
            "tool_name": "memory_read",
            "summary": "Tool failed."
        });

        assert_eq!(
            tool_call_finished_summary(&data, "memory_read", true),
            "Read memory failed."
        );
    }

    #[test]
    fn tool_call_finished_summary_replaces_provider_name_finished_summary_for_task_lists() {
        let data = json!({
            "tool_name": "list_task_lists",
            "summary": "Finished list_task_lists"
        });

        assert_eq!(
            tool_call_finished_summary(&data, "list_task_lists", false),
            "Listed task lists."
        );
    }

    #[test]
    fn tool_call_finished_summary_replaces_provider_name_finished_summary_for_common_den_tools() {
        for (tool_name, expected) in [
            ("session_info", "Inspected session."),
            ("set_conversation_title", "Set conversation title."),
            ("memory_read", "Read memory."),
            ("memory_search", "Searched memory."),
            ("web_search", "Searched web."),
            ("git_status", "Checked git status."),
        ] {
            let data = json!({
                "tool_name": tool_name,
                "summary": format!("Finished {tool_name}")
            });

            assert_eq!(
                tool_call_finished_summary(&data, tool_name, false),
                expected
            );
        }
    }

    #[test]
    fn tool_call_finished_summary_adds_command_name_for_generic_run_command_summary() {
        let data = json!({
            "tool_name": "run_command",
            "summary": "Tool completed.",
            "args": {
                "command": "cargo",
                "args": ["test"]
            }
        });

        let summary = tool_call_finished_summary(&data, "run_command", false);

        assert!(summary.contains("Run Command completed."), "{summary}");
        assert!(summary.contains("Command: `cargo test`."), "{summary}");
    }

    #[test]
    fn tool_call_finished_summary_adds_git_subcommand_for_generic_summary() {
        let data = json!({
            "tool_name": "process_run",
            "summary": "Tool completed.",
            "args": {
                "command": "git",
                "args": ["status", "--short"]
            }
        });

        let summary = tool_call_finished_summary(&data, "process_run", false);

        assert!(summary.contains("Run process completed."), "{summary}");
        assert!(summary.contains("Command: `git status`."), "{summary}");
    }

    #[test]
    fn successful_tool_result_params_do_not_put_diagnostic_in_error_field() {
        let config = Config {
            api_url: "http://den.test".to_string(),
            bear: "meta".to_string(),
            token: "token".to_string(),
            client: "zed".to_string(),
        };
        let params = tool_result_rpc_params(
            &config,
            "session-1",
            "run-1",
            "call-1",
            &json!({
                "tool_name": "fs_read_text_file",
                "status": "ok",
                "content": "",
                "structured_content": { "content": "hello" },
                "diagnostic": { "phase": "permission_local_tool_completed" }
            }),
        );

        assert_eq!(params["tool_name"], "fs_read_text_file");
        assert_eq!(
            params["diagnostic"]["phase"],
            "permission_local_tool_completed"
        );
        assert!(params["error"].is_null(), "{params}");
    }

    #[test]
    fn failed_tool_result_params_copy_diagnostic_to_error_field() {
        let config = Config {
            api_url: "http://den.test".to_string(),
            bear: "meta".to_string(),
            token: "token".to_string(),
            client: "zed".to_string(),
        };
        let params = tool_result_rpc_params(
            &config,
            "session-1",
            "run-1",
            "call-1",
            &json!({
                "tool_name": "fs_read_text_file",
                "status": "error",
                "content": "failed",
                "diagnostic": { "phase": "permission_local_tool_failed" }
            }),
        );

        assert_eq!(params["error"]["phase"], "permission_local_tool_failed");
    }

    #[test]
    fn tool_call_finished_summary_for_placeholder_tool_exposes_details() {
        let data = json!({
            "tool_call_id": "call-unknown-1",
            "summary": "Tool completed.",
            "result": { "count": 3 }
        });

        let summary = tool_call_finished_summary(&data, "tool", false);

        assert!(summary.contains("Tool call completed"), "{summary}");
        assert!(summary.contains("call-unknown-1"), "{summary}");
        assert!(summary.contains("\"count\":3"), "{summary}");
        assert!(!matches!(
            summary.as_str(),
            "Tool completed." | "Tool failed."
        ));
    }

    #[test]
    fn tool_call_finished_summary_prefers_specific_error_detail() {
        let data = json!({
            "tool_name": "memory_read",
            "summary": "Tool failed.",
            "diagnostic": {
                "error": "permission denied reading pair/notes.md"
            }
        });

        assert_eq!(
            tool_call_finished_summary(&data, "memory_read", true),
            "permission denied reading pair/notes.md"
        );
    }

    #[test]
    fn bearwire_plan_update_entries_accepts_runtime_progress_detail() {
        let event = json!({
            "type": "run.progress",
            "data": {
                "kind": "plan_update",
                "text": null,
                "phase": "tool_result",
                "detail": {
                    "entries": [
                        { "id": "inspect", "title": "Inspect logs", "status": "completed" },
                        { "id": "patch", "title": "Patch ACP plan projection", "status": "in_progress" }
                    ]
                }
            }
        });

        let entries = bearwire_plan_update_entries(&event);

        assert_eq!(entries.as_array().map(Vec::len), Some(2));
        assert_eq!(entries[0]["title"], "Inspect logs");
        assert_eq!(entries[1]["status"], "in_progress");
    }

    #[test]
    fn bearwire_plan_update_entries_accepts_nested_plan_items() {
        let event = json!({
            "type": "run.progress",
            "data": {
                "kind": "plan_update",
                "detail": {
                    "plan": {
                        "items": [
                            { "id": "one", "title": "One", "status": "pending" }
                        ]
                    }
                }
            }
        });

        let entries = bearwire_plan_update_entries(&event);

        assert_eq!(entries.as_array().map(Vec::len), Some(1));
        assert_eq!(entries[0]["id"], "one");
    }
}
