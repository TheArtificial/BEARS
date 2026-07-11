use anyhow::{anyhow, Context, Result};

use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use crate::{
    adapter_contract_context, den_request_context, env_bool,
    handle_conversation_resolved_projection, handle_permission_request_event,
    handle_plan_update_projection, handle_session_info_projection, handle_status_text_for_turn,
    plan_entries_from_plan_update_event, send_agent_message_chunk_for_turn,
    send_agent_thought_chunk_for_turn, send_tool_call_update, spawn_tool_request_task,
    stream_has_successful_terminal_condition, truncate_for_log, AdapterSharedState, AdapterState,
    Config, SseFrameOutcome, SseStreamDiagnostics, ToolCallUpdatePayload,
};

const BEARWIRE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const BEARWIRE_PROMPT_TIMEOUT: Duration = Duration::from_secs(600);
const BEARWIRE_EVENT_FETCH_MAX_CONSECUTIVE_ERRORS: usize = 5;
const BEARWIRE_TOOL_RAW_OUTPUT_PREVIEW_CHARS: usize = 24 * 1024;
const BEARWIRE_OBLIGATION_SYNC_INTERVAL: Duration = Duration::from_secs(1);
const BEARWIRE_RUN_STATE_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(5);

fn recoverable_read_only_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "fs_read_text_file" | "fs_list_directory" | "fs_find_paths" | "fs_search_files" | "fs_stat"
    )
}

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
                    .pointer("/tool_call/id")
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
    let mut last_obligation_sync: Option<Instant> = None;
    let mut last_run_state_diagnostic_log = Instant::now();
    let mut last_run_state_summary: Option<String> = None;
    let mut logged_initial_wait = false;
    let mut consecutive_fetch_errors = 0usize;

    while started.elapsed() < BEARWIRE_PROMPT_TIMEOUT {
        let replay = match fetch_events(http, config, session_id, after).await {
            Ok(replay) => {
                consecutive_fetch_errors = 0;
                replay
            }
            Err(err) => {
                consecutive_fetch_errors += 1;
                diagnostics.fetch_errors += 1;
                tracing::warn!(
                    target: "bear_armature::lifecycle",
                    session_id,
                    run_id,
                    after = ?after,
                    consecutive_fetch_errors,
                    error = %err,
                    "BearWire event fetch failed; retrying before aborting prompt loop"
                );
                if consecutive_fetch_errors >= BEARWIRE_EVENT_FETCH_MAX_CONSECUTIVE_ERRORS {
                    return Err(err).context("BearWire event fetch failed repeatedly");
                }
                if run_id != "<unknown>" {
                    match fetch_run_state(http, config, session_id, run_id).await {
                        Ok(state) => {
                            service_run_state_tool_obligations(
                                config,
                                shared_state,
                                session_id,
                                run_id,
                                &state,
                                turn_token,
                            )
                            .await;
                        }
                        Err(state_err) => {
                            tracing::debug!(
                                target: "bear_armature::lifecycle",
                                session_id,
                                run_id,
                                error = %state_err,
                                "BearWire run.state obligation sync failed after event fetch error"
                            );
                        }
                    }
                }
                sleep(BEARWIRE_POLL_INTERVAL).await;
                continue;
            }
        };
        let replay_count = replay.frames.len();
        let next_after = replay.next_after;
        for frame in replay.frames {
            let _sequence = frame.sequence;
            let Some(event) = frame.event else {
                continue;
            };
            let outcome = match handle_bearwire_event(
                config,
                adapter_state,
                shared_state,
                session_id,
                run_id,
                &event,
                &mut diagnostics,
                turn_token,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(err) if event.get("type").and_then(Value::as_str) == Some("run.failed") => {
                    return Err(err);
                }
                Err(err) => {
                    diagnostics.event_errors += 1;
                    tracing::warn!(
                        target: "bear_armature::lifecycle",
                        session_id,
                        run_id,
                        event_type = event.get("type").and_then(|value| value.as_str()).unwrap_or("<missing>"),
                        error = %err,
                        sample = %truncate_for_log(&event.to_string(), 360),
                        "BearWire event handling failed; skipping non-terminal event"
                    );
                    continue;
                }
            };
            saw_done |= outcome.saw_done;
            saw_visible_output |= outcome.saw_visible_output;
            saw_tool_activity |= outcome.saw_tool_activity;
            saw_error |= outcome.saw_error;
            if saw_done {
                break;
            }
        }
        after = next_after;
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
        let should_sync_obligations = run_id != "<unknown>"
            && last_obligation_sync
                .map(|last_sync| last_sync.elapsed() >= BEARWIRE_OBLIGATION_SYNC_INTERVAL)
                .unwrap_or(true);
        if should_sync_obligations {
            last_obligation_sync = Some(Instant::now());
            match fetch_run_state(http, config, session_id, run_id).await {
                Ok(state) => {
                    if let Some(summary) = run_state_obligation_summary(&state) {
                        let should_log_summary = last_run_state_summary.as_deref()
                            != Some(summary.as_str())
                            || last_run_state_diagnostic_log.elapsed()
                                >= BEARWIRE_RUN_STATE_DIAGNOSTIC_INTERVAL;
                        if should_log_summary {
                            last_run_state_diagnostic_log = Instant::now();
                            tracing::warn!(
                                target: "bear_armature::lifecycle",
                                session_id,
                                run_id,
                                summary = %summary,
                                "BearWire run.state reports active client obligations"
                            );
                            if crate::bear_debug_verbose() {
                                eprintln!(
                                    "bear-armature: BearWire run.state obligations session_id={} run_id={} {}",
                                    session_id, run_id, summary
                                );
                            }
                            last_run_state_summary = Some(summary);
                        }
                    } else {
                        last_run_state_summary = None;
                    }
                    service_run_state_tool_obligations(
                        config,
                        shared_state,
                        session_id,
                        run_id,
                        &state,
                        turn_token,
                    )
                    .await;
                }
                Err(err) => {
                    tracing::debug!(
                        target: "bear_armature::lifecycle",
                        session_id,
                        run_id,
                        error = %err,
                        "BearWire run.state obligation sync failed"
                    );
                }
            }
        }
        sleep(BEARWIRE_POLL_INTERVAL).await;
    }

    if !stream_has_successful_terminal_condition(
        saw_visible_output,
        saw_error,
        saw_done,
        saw_tool_activity,
    ) {
        let reason = if saw_tool_activity {
            "BEARS BearWire stream ended after tool activity but before a terminal run event or assistant output"
        } else {
            "BEARS BearWire stream ended without visible output, tool activity, or an error"
        };
        return Err(anyhow!("{reason}. Diagnostics: {}", diagnostics.summary()));
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

async fn fetch_run_state(
    http: &reqwest::Client,
    config: &Config,
    session_id: &str,
    run_id: &str,
) -> Result<Value> {
    rpc_call(
        http,
        config,
        "run.state",
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "run_id": run_id,
            "limit": 20,
        }),
    )
    .await
}

fn run_state_obligation_summary(state: &Value) -> Option<String> {
    let run_state = state
        .pointer("/run/state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let open = state
        .get("open_obligations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if open.is_empty() {
        return None;
    }
    let obligations = open
        .iter()
        .take(5)
        .map(|obligation| {
            let id = obligation
                .get("id")
                .or_else(|| obligation.get("obligation_id"))
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let state = obligation
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let kind = obligation
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let expected = obligation
                .get("expected_responder_action")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let tool_call = obligation
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("<none>");
            let permission = obligation
                .get("permission_id")
                .and_then(Value::as_str)
                .unwrap_or("<none>");
            let updated = obligation
                .get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            format!(
                "id={id} state={state} kind={kind} expected={expected} tool_call={tool_call} permission={permission} updated_at={updated}"
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "run_state={run_state} open_obligation_count={} open_obligations=[{}]",
        open.len(),
        obligations
    ))
}

fn actionable_read_only_tool_request_event_from_obligation(
    run_id: &str,
    obligation: &Value,
) -> Option<Value> {
    let expected = obligation
        .get("expected_responder_action")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let kind = obligation
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if expected != "tool_result" && kind != "tool_result" {
        return None;
    }
    let state = obligation
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(state, "requested" | "waiting_for_client") {
        return None;
    }
    let request = obligation
        .get("request_payload")
        .filter(|value| value.is_object())
        .unwrap_or(obligation);
    if request
        .get("approval_required")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let execution_target = request
        .get("execution_target")
        .or_else(|| obligation.get("execution_target"))
        .and_then(Value::as_str)
        .unwrap_or("armature_local");
    if execution_target == "den" {
        return None;
    }
    let tool_name = request
        .get("tool_name")
        .or_else(|| obligation.get("tool_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if !recoverable_read_only_tool(tool_name) {
        return None;
    }
    let tool_call_id = request
        .get("tool_call_id")
        .or_else(|| obligation.get("tool_call_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let arguments = request
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    Some(json!({
        "type": "tool_call.requested",
        "run_id": run_id,
        "data": {
            "tool_call": {
                "id": tool_call_id,
                "name": tool_name,
                "kind": "function",
                "arguments": arguments,
            },
            "approval_required": false,
            "execution_target": "armature_local",
            "serviced_from_run_state": true,
        }
    }))
}

async fn service_run_state_tool_obligations(
    config: &Config,
    shared_state: &AdapterSharedState,
    session_id: &str,
    run_id: &str,
    state: &Value,
    turn_token: Uuid,
) {
    let Some(open) = state.get("open_obligations").and_then(Value::as_array) else {
        return;
    };
    for obligation in open {
        let Some(event) =
            actionable_read_only_tool_request_event_from_obligation(run_id, obligation)
        else {
            continue;
        };
        let tool_call_id = event
            .pointer("/data/tool_call/id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let tool_name = event
            .pointer("/data/tool_call/name")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        if shared_state
            .tool_tasks
            .get(session_id, tool_call_id)
            .await
            .is_some()
        {
            continue;
        }
        tracing::warn!(
            target: "bear_armature::lifecycle",
            session_id,
            run_id,
            tool_call_id,
            tool_name,
            "servicing read-only local tool obligation from BearWire run.state"
        );
        spawn_tool_request_task(
            config.clone(),
            shared_state.clone(),
            session_id.to_string(),
            event,
            turn_token,
        );
    }
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
    next_after: Option<i64>,
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
    fetch_event_page(http, config, session_id, after).await
}

async fn fetch_event_page(
    http: &reqwest::Client,
    config: &Config,
    session_id: &str,
    after: Option<i64>,
) -> Result<BearWireReplay> {
    let mut url = format!(
        "{}/bearwire/v1/sessions/{}/events/page?bear_slug={}",
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
        return Err(anyhow!(
            "BearWire event page HTTP {status}: {}",
            body.trim()
        ));
    }
    let body = response.text().await.unwrap_or_default();
    parse_event_page(&body, after)
        .with_context(|| format!("parse BearWire event page JSON: {body}"))
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

#[derive(Debug, Deserialize)]
struct BearWireToolCallCard {
    id: String,
    name: Option<String>,
    #[serde(default)]
    arguments: Option<Value>,
    #[serde(default)]
    display: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct BearWireToolCallFinishedData {
    tool_call: BearWireToolCallCard,
}

impl BearWireToolCallFinishedData {
    fn parse(event: &Value) -> Result<Self> {
        let data = event
            .get("data")
            .cloned()
            .ok_or_else(|| anyhow!("BearWire tool completion missing data"))?;
        serde_json::from_value(data).context("parse canonical BearWire tool completion data")
    }
}

async fn handle_bearwire_tool_call_finished_event(
    shared_state: &AdapterSharedState,
    session_id: &str,
    event: &Value,
    failed: bool,
) -> Result<()> {
    let data = event.get("data").unwrap_or(&Value::Null);
    let canonical = BearWireToolCallFinishedData::parse(event)?;
    let lookup_tool_call_id = canonical.tool_call.id.as_str();
    let cached = shared_state
        .tool_tasks
        .get(session_id, lookup_tool_call_id)
        .await;
    let cached_input_args = cached.as_ref().and_then(|record| record.input_args.clone());
    let had_cached_start = cached.is_some();
    let tool_call_id = canonical.tool_call.id;
    let tool_name = canonical
        .tool_call
        .name
        .as_deref()
        .filter(|name| !crate::is_placeholder_tool_name(name))
        .map(str::to_string)
        .or_else(|| cached.as_ref().map(|record| record.tool_name.clone()))
        .ok_or_else(|| anyhow!("canonical BearWire tool completion missing tool_call.name"))?;
    let summary = tool_call_finished_summary(data, &tool_name, failed);
    let status = if failed { "failed" } else { "completed" };
    let mut projection_event = json!({
        "run_id": event.get("run_id").and_then(Value::as_str),
        "data": {
            "tool_call": {
                "id": tool_call_id,
                "name": tool_name.clone(),
            }
        }
    });
    if let Some(args) = cached_input_args.or(canonical.tool_call.arguments) {
        projection_event["data"]["tool_call"]["arguments"] = args;
    }
    if let Some(display) = canonical.tool_call.display {
        projection_event["data"]["tool_call"]["display"] = display;
    }
    send_tool_call_update(
        session_id,
        &tool_call_id,
        &tool_name,
        ToolCallUpdatePayload {
            status,
            text: &summary,
            event: Some(&projection_event),
            raw_output: Some(compact_json_preview(
                data,
                BEARWIRE_TOOL_RAW_OUTPUT_PREVIEW_CHARS,
            )),
            extra_content: Vec::new(),
        },
    )
    .await?;
    if had_cached_start {
        shared_state
            .tool_tasks
            .set_phase(
                session_id,
                &tool_call_id,
                &tool_name,
                crate::ToolTaskPhase::ResultPosted,
            )
            .await;
        let _ = shared_state
            .tool_tasks
            .remove(session_id, &tool_call_id)
            .await;
    }
    Ok(())
}

fn bearwire_message_delta_text(event: &Value) -> &str {
    event
        .get("data")
        .and_then(|data| {
            data.get("delta")
                .or_else(|| data.get("text"))
                .or_else(|| data.get("reasoning"))
                .or_else(|| data.get("thinking"))
                .or_else(|| data.get("thought"))
        })
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn value_has_reasoning_marker(value: &Value) -> bool {
    value
        .as_str()
        .map(|raw| {
            let raw = raw.to_ascii_lowercase();
            raw.contains("reasoning") || raw.contains("thinking") || raw.contains("thought")
        })
        .unwrap_or(false)
}

fn bearwire_message_delta_is_reasoning(event: &Value) -> bool {
    let Some(data) = event.get("data") else {
        return false;
    };
    [
        data.get("kind"),
        data.get("role"),
        data.get("type"),
        data.get("message_type"),
        data.get("channel"),
        data.get("source"),
        data.get("part_kind"),
        data.pointer("/delta/kind"),
        data.pointer("/delta/role"),
        data.pointer("/delta/type"),
    ]
    .into_iter()
    .flatten()
    .any(value_has_reasoning_marker)
        || data.get("reasoning").is_some()
        || data.get("thinking").is_some()
        || data.get("thought").is_some()
}

fn parse_event_page(body: &str, after: Option<i64>) -> Result<BearWireReplay> {
    let value: Value = serde_json::from_str(body)?;
    let next_after = value.get("next_after").and_then(Value::as_i64).or(after);
    let frames = value
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .map(|entry| BearWireFrame {
                    sequence: entry.get("sequence").and_then(Value::as_i64),
                    event: entry.get("event").cloned(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(BearWireReplay { frames, next_after })
}

fn event_run_id(event: &Value) -> Option<&str> {
    event
        .get("run_id")
        .and_then(Value::as_str)
        .or_else(|| event.pointer("/data/run_id").and_then(Value::as_str))
}

fn event_is_run_scoped(ty: &str) -> bool {
    ty.starts_with("run.")
        || ty.starts_with("message.")
        || ty.starts_with("tool_call.")
        || matches!(
            ty,
            "client.waiting"
                | "permission.requested"
                | "permission.granted"
                | "permission.denied"
                | "permission.expired"
        )
}

async fn handle_bearwire_event(
    config: &Config,
    adapter_state: &mut AdapterState,
    shared_state: &AdapterSharedState,
    session_id: &str,
    current_run_id: &str,
    event: &Value,
    diagnostics: &mut SseStreamDiagnostics,
    turn_token: Uuid,
) -> Result<SseFrameOutcome> {
    diagnostics.frames += 1;
    let outcome = SseFrameOutcome::default();
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    if current_run_id != "<unknown>" && event_is_run_scoped(ty) {
        if let Some(event_run_id) = event_run_id(event) {
            if event_run_id != current_run_id {
                tracing::debug!(
                    target: "bear_armature::lifecycle",
                    session_id,
                    current_run_id,
                    event_run_id,
                    event_type = ty,
                    "ignoring BearWire event for non-current run"
                );
                return Ok(outcome);
            }
        }
    }
    let mut outcome = outcome;
    diagnostics.observe_event(event);

    match ty {
        "message.delta" => {
            let text = bearwire_message_delta_text(event);
            if bearwire_message_delta_is_reasoning(event) {
                if !text.is_empty() {
                    if crate::bear_debug_verbose() {
                        eprintln!(
                            "bear-armature: reclassified reasoning-tagged message.delta as thought session_id={} sample={}",
                            session_id,
                            truncate_for_log(text, 160)
                        );
                    }
                    send_agent_thought_chunk_for_turn(shared_state, session_id, turn_token, text)
                        .await?;
                }
            } else {
                outcome.saw_visible_output = !text.is_empty();
                if !text.is_empty() {
                    send_agent_message_chunk_for_turn(shared_state, session_id, turn_token, text)
                        .await?;
                }
            }
        }
        "message.reasoning.delta" => {
            let text = bearwire_message_delta_text(event);
            if !text.is_empty() {
                send_agent_thought_chunk_for_turn(shared_state, session_id, turn_token, text)
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
                let entries_json = bearwire_plan_update_entries(event);
                let plan_event = json!({ "entries": entries_json });
                let entries = plan_entries_from_plan_update_event(&plan_event);
                let approval_fallback = data
                    .get("approval_fallback")
                    .or_else(|| data.pointer("/detail/approval_fallback"));
                outcome.saw_visible_output = true;
                outcome.saw_tool_activity = true;
                diagnostics.saw_tool_activity = true;
                handle_plan_update_projection(
                    shared_state,
                    session_id,
                    turn_token,
                    entries,
                    approval_fallback,
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
                handle_status_text_for_turn(shared_state, session_id, turn_token, text).await?;
            }
        }
        "session_info_update" => {
            let data = event.get("data").unwrap_or(&Value::Null);
            let title = event
                .get("title")
                .or_else(|| data.get("title"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let updated_at = event
                .get("updated_at")
                .or_else(|| data.get("updated_at"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let context_budget = event
                .pointer("/meta/bears/context_budget")
                .cloned()
                .or_else(|| event.pointer("/_meta/bears/context_budget").cloned())
                .or_else(|| data.pointer("/meta/bears/context_budget").cloned())
                .or_else(|| data.get("context_budget").cloned());
            let runtime = event
                .pointer("/meta/bears/runtime")
                .cloned()
                .or_else(|| event.pointer("/_meta/bears/runtime").cloned())
                .or_else(|| data.pointer("/meta/bears/runtime").cloned())
                .or_else(|| data.get("runtime").cloned());
            handle_session_info_projection(
                adapter_state,
                shared_state,
                session_id,
                title,
                updated_at,
                context_budget,
                runtime,
            )
            .await?;
        }
        "session.bound" => {
            let conversation_id = event
                .pointer("/data/binding/conversation_id")
                .or_else(|| event.pointer("/data/conversation_id"))
                .and_then(Value::as_str);
            if let Some(conversation_id) = conversation_id {
                handle_conversation_resolved_projection(
                    config,
                    adapter_state,
                    shared_state,
                    session_id,
                    conversation_id,
                )
                .await?;
            }
        }
        "run.completed" => {
            tracing::info!(
                target: "bear_armature::lifecycle",
                session_id,
                run_id = event.get("run_id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                "BearWire run.completed received"
            );
            outcome.saw_done = true;
        }
        "run.failed" => {
            tracing::warn!(
                target: "bear_armature::lifecycle",
                session_id,
                run_id = event.get("run_id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                reason = event.pointer("/data/reason").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                "BearWire run.failed received"
            );
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
            tracing::info!(
                target: "bear_armature::lifecycle",
                session_id,
                run_id = event.get("run_id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                tool_call_id = event.pointer("/data/tool_call/id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                tool_name = event.pointer("/data/tool_call/name").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                "BearWire tool_call.requested received"
            );
            outcome.saw_tool_activity = true;
            diagnostics.saw_tool_activity = true;
            spawn_tool_request_task(
                config.clone(),
                shared_state.clone(),
                session_id.to_string(),
                event.clone(),
                turn_token,
            );
        }
        "tool_call.completed" | "tool_call.warning" | "tool_call.cancelled" => {
            tracing::info!(
                target: "bear_armature::lifecycle",
                session_id,
                run_id = event.get("run_id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                tool_call_id = event.pointer("/data/tool_call/id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                status = ty,
                "BearWire tool_call terminal update received"
            );
            outcome.saw_tool_activity = true;
            diagnostics.saw_tool_activity = true;
            handle_bearwire_tool_call_finished_event(shared_state, session_id, event, false)
                .await?;
        }
        "tool_call.failed" => {
            tracing::warn!(
                target: "bear_armature::lifecycle",
                session_id,
                run_id = event.get("run_id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                tool_call_id = event.pointer("/data/tool_call/id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                "BearWire tool_call.failed received"
            );
            outcome.saw_tool_activity = true;
            outcome.saw_error = true;
            diagnostics.saw_tool_activity = true;
            diagnostics.saw_error = true;
            handle_bearwire_tool_call_finished_event(shared_state, session_id, event, true).await?;
        }
        "client.waiting" => {
            tracing::info!(
                target: "bear_armature::lifecycle",
                session_id,
                run_id = event.get("run_id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                tool_call_id = event.pointer("/data/tool_call/id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                tool_name = event.pointer("/data/tool_call/name").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                permission_id = event.pointer("/data/permission/id").and_then(|value| value.as_str()).unwrap_or("<unknown>"),
                "BearWire client.waiting received"
            );
            outcome.saw_tool_activity = true;
            outcome.saw_visible_output = true;
            diagnostics.saw_tool_activity = true;
            diagnostics.saw_visible_output = true;
            handle_permission_request_event(
                config,
                adapter_state,
                shared_state,
                &shared_state.mcp_registry,
                session_id,
                event,
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
    fn parse_event_page_uses_server_owned_cursor() {
        let page = json!({
            "ok": true,
            "events": [
                {
                    "sequence": 42,
                    "event": {"type": "run.progress", "run_id": "run-1", "data": {}}
                }
            ],
            "next_after": 100,
            "has_more": true
        });

        let parsed = parse_event_page(&page.to_string(), Some(41)).unwrap();

        assert_eq!(parsed.next_after, Some(100));
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].sequence, Some(42));
        assert_eq!(
            parsed.frames[0].event.as_ref().unwrap()["type"],
            "run.progress"
        );
    }

    #[test]
    fn parse_event_page_does_not_advance_without_next_after() {
        let page = json!({
            "ok": true,
            "events": [],
            "has_more": false
        });

        let parsed = parse_event_page(&page.to_string(), Some(41)).unwrap();

        assert_eq!(parsed.next_after, Some(41));
        assert!(parsed.frames.is_empty());
    }

    #[test]
    fn run_scoped_event_helpers_identify_foreign_terminal_events() {
        let event = json!({
            "type": "run.failed",
            "run_id": "run-old",
            "data": { "reason": "client_obligation_timeout" }
        });

        assert!(event_is_run_scoped("run.failed"));
        assert_eq!(event_run_id(&event), Some("run-old"));
        assert_ne!(event_run_id(&event), Some("run-current"));
        assert!(!event_is_run_scoped("session.bound"));
    }

    #[test]
    fn message_delta_with_reasoning_metadata_is_thought_not_visible_output() {
        let event = json!({
            "type": "message.delta",
            "data": {
                "kind": "reasoning_delta",
                "delta": "thinking about the next step"
            }
        });

        assert!(bearwire_message_delta_is_reasoning(&event));
        assert_eq!(
            bearwire_message_delta_text(&event),
            "thinking about the next step"
        );
    }

    #[test]
    fn message_delta_without_reasoning_metadata_is_visible_text() {
        let event = json!({
            "type": "message.delta",
            "data": {
                "delta": "hello user"
            }
        });

        assert!(!bearwire_message_delta_is_reasoning(&event));
        assert_eq!(bearwire_message_delta_text(&event), "hello user");
    }

    #[test]
    fn reasoning_text_fallback_is_extracted_from_compatibility_payload() {
        let event = json!({
            "type": "message.delta",
            "data": {
                "source": "provider_reasoning",
                "reasoning": "compat reasoning"
            }
        });

        assert!(bearwire_message_delta_is_reasoning(&event));
        assert_eq!(bearwire_message_delta_text(&event), "compat reasoning");
    }

    #[test]
    fn bearwire_finished_tool_requires_canonical_tool_call_identity() {
        let event = json!({
            "type": "tool_call.completed",
            "run_id": "run-1",
            "data": {
                "tool_call": {
                    "id": "call-1",
                    "name": "fs_read_text_file",
                    "title": "Read file",
                    "arguments": { "path": "/workspace/README.md" }
                },
                "summary": "Read file."
            }
        });

        let parsed = BearWireToolCallFinishedData::parse(&event).unwrap();

        assert_eq!(parsed.tool_call.id, "call-1");
        assert_eq!(parsed.tool_call.name.as_deref(), Some("fs_read_text_file"));
        assert_eq!(
            parsed.tool_call.arguments.as_ref().unwrap()["path"],
            "/workspace/README.md"
        );
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
    fn run_state_obligation_summary_reports_open_obligations() {
        let state = json!({
            "run": { "state": "waiting_for_tool_result" },
            "open_obligations": [{
                "id": "obl-1",
                "kind": "tool_result",
                "expected_responder_action": "tool_result",
                "state": "waiting_for_client",
                "tool_call_id": "call-1",
                "permission_id": null,
                "updated_at": "2026-07-10T00:00:00Z"
            }]
        });

        let summary = run_state_obligation_summary(&state).unwrap();

        assert!(summary.contains("waiting_for_tool_result"), "{summary}");
        assert!(summary.contains("obl-1"), "{summary}");
        assert!(summary.contains("call-1"), "{summary}");
        assert!(summary.contains("waiting_for_client"), "{summary}");
    }

    #[test]
    fn run_state_obligation_summary_accepts_continuation_obligation_shape() {
        let state = json!({
            "run": { "state": "waiting_for_tool_result" },
            "open_obligations": [{
                "obligation_id": "obl-2",
                "kind": "tool_result",
                "expected_responder_action": "tool_result",
                "state": "waiting_for_client",
                "tool_call_id": "call-2",
                "permission_id": null
            }]
        });

        let summary = run_state_obligation_summary(&state).unwrap();

        assert!(summary.contains("obl-2"), "{summary}");
        assert!(summary.contains("call-2"), "{summary}");
    }

    #[test]
    fn run_state_obligation_summary_ignores_clean_state() {
        let state = json!({ "run": { "state": "running" }, "open_obligations": [] });
        assert!(run_state_obligation_summary(&state).is_none());
    }

    #[test]
    fn reconstructs_actionable_read_only_tool_request_from_run_state() {
        let obligation = json!({
            "id": "obl-1",
            "kind": "tool_result",
            "expected_responder_action": "tool_result",
            "state": "waiting_for_client",
            "tool_call_id": "call-search",
            "request_payload": {
                "tool_call_id": "call-search",
                "tool_name": "fs_search_files",
                "arguments": { "path": "docs", "query": "BearWire", "limit": 100 },
                "approval_required": false,
                "execution_target": "armature_local"
            }
        });

        let event = actionable_read_only_tool_request_event_from_obligation("run-1", &obligation)
            .expect("actionable read-only tool");

        assert_eq!(event["type"], "tool_call.requested");
        assert_eq!(event["run_id"], "run-1");
        assert_eq!(event["data"]["tool_call"]["id"], "call-search");
        assert_eq!(event["data"]["tool_call"]["name"], "fs_search_files");
        assert_eq!(event["data"]["tool_call"]["arguments"]["query"], "BearWire");
        assert_eq!(event["data"]["serviced_from_run_state"], true);
    }

    #[test]
    fn does_not_service_mutating_or_approval_obligations_from_run_state() {
        let mutating = json!({
            "kind": "tool_result",
            "expected_responder_action": "tool_result",
            "state": "waiting_for_client",
            "request_payload": {
                "tool_call_id": "call-edit",
                "tool_name": "fs_edit_file",
                "arguments": { "path": "README.md" },
                "approval_required": false,
                "execution_target": "armature_local"
            }
        });
        assert!(
            actionable_read_only_tool_request_event_from_obligation("run-1", &mutating).is_none()
        );

        let approval = json!({
            "kind": "tool_result",
            "expected_responder_action": "tool_result",
            "state": "waiting_for_client",
            "request_payload": {
                "tool_call_id": "call-read",
                "tool_name": "fs_read_text_file",
                "arguments": { "path": "README.md" },
                "approval_required": true,
                "execution_target": "armature_local"
            }
        });
        assert!(
            actionable_read_only_tool_request_event_from_obligation("run-1", &approval).is_none()
        );
    }

    #[test]
    fn does_not_service_den_owned_obligations_from_run_state() {
        let obligation = json!({
            "kind": "tool_result",
            "expected_responder_action": "tool_result",
            "state": "waiting_for_client",
            "request_payload": {
                "tool_call_id": "call-title",
                "tool_name": "set_conversation_title",
                "arguments": { "title": "Test" },
                "approval_required": false,
                "execution_target": "den"
            }
        });
        assert!(
            actionable_read_only_tool_request_event_from_obligation("run-1", &obligation).is_none()
        );
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
    fn set_conversation_title_finished_summary_is_specific_for_acp_tool_card() {
        let data = json!({
            "tool_call": {
                "id": "call-title",
                "name": "set_conversation_title",
                "arguments": { "title": "Test Armature ACP conversation title tool" }
            },
            "summary": "Finished set_conversation_title"
        });

        assert_eq!(
            tool_call_finished_summary(&data, "set_conversation_title", false),
            "Set conversation title."
        );
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
