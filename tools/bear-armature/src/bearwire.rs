use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

use crate::{
    adapter_contract_context, den_request_context, env_bool, handle_den_event,
    stream_has_successful_terminal_condition, truncate_for_log, AdapterSharedState, AdapterState,
    Config, SseFrameOutcome, SseStreamDiagnostics,
};

const BEARWIRE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const BEARWIRE_PROMPT_TIMEOUT: Duration = Duration::from_secs(600);

fn bearwire_env_value() -> Option<String> {
    std::env::var("BEARS_BEARWIRE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
}

pub(crate) fn legacy_acp_http_forced() -> bool {
    env_bool("BEARS_LEGACY_ACP_HTTP")
}

pub(crate) fn enabled() -> bool {
    if legacy_acp_http_forced() {
        return false;
    }
    match bearwire_env_value().as_deref() {
        None | Some("") | Some("auto") => true,
        Some("0" | "false" | "no" | "off" | "disabled") => false,
        Some(_) => env_bool("BEARS_BEARWIRE"),
    }
}

pub(crate) fn required() -> bool {
    env_bool("BEARS_BEARWIRE_REQUIRED") && !legacy_acp_http_forced()
}

pub(crate) fn mode_summary() -> String {
    let raw = std::env::var("BEARS_BEARWIRE").unwrap_or_else(|_| "<unset>".to_string());
    let legacy_raw =
        std::env::var("BEARS_LEGACY_ACP_HTTP").unwrap_or_else(|_| "<unset>".to_string());
    let mode = if legacy_acp_http_forced() {
        "legacy-forced"
    } else if required() {
        "required"
    } else if raw.trim().is_empty() || raw == "<unset>" || raw.trim().eq_ignore_ascii_case("auto") {
        "auto"
    } else if enabled() {
        "enabled"
    } else {
        "disabled"
    };
    format!(
        "{mode} (BEARS_BEARWIRE={raw}, BEARS_BEARWIRE_REQUIRED={}, BEARS_LEGACY_ACP_HTTP={legacy_raw})",
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

pub(crate) async fn handle_prompt(
    http: &reqwest::Client,
    config: &Config,
    adapter_state: &mut AdapterState,
    shared_state: &AdapterSharedState,
    response_id: Value,
    session_id: &str,
    prompt: &str,
    client_context: Value,
    conversation_id: Option<&str>,
    requested_mode: &str,
    turn_token: Uuid,
) -> Result<()> {
    let cwd = client_context
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let session_result = rpc_call(
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
            "client_context": client_context.clone(),
        }),
    )
    .await
    .context("BearWire session.open failed")?;

    eprintln!(
        "bear-armature: BearWire session.open ok session_id={} result={}",
        session_id,
        truncate_for_log(&session_result.to_string(), 360)
    );

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
    eprintln!(
        "bear-armature: BearWire run.start accepted session_id={} run_id={} after={:?}",
        session_id, run_id, after
    );

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
            eprintln!(
                "bear-armature: BearWire run terminal event received session_id={} run_id={} diagnostics={}",
                session_id,
                run_id,
                diagnostics.summary()
            );
            break;
        }
        if !logged_initial_wait
            && started.elapsed() >= Duration::from_secs(5)
            && !saw_visible_output
            && !saw_tool_activity
            && !saw_error
        {
            logged_initial_wait = true;
            eprintln!(
                "bear-armature: BearWire still waiting for first visible/tool event session_id={} run_id={} after={:?} elapsed_ms={} diagnostics={}",
                session_id,
                run_id,
                after,
                started.elapsed().as_millis(),
                diagnostics.summary()
            );
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
        json!({
            "bear_slug": config.bear,
            "session_id": session_id,
            "run_id": run_id,
            "tool_call_id": tool_call_id,
            "status": payload.get("status").and_then(Value::as_str).unwrap_or("ok"),
            "content": payload.get("content").cloned().unwrap_or(Value::Null),
            "structured_content": payload.get("structured_content").cloned().unwrap_or(Value::Null),
            "error": payload.get("error").cloned().unwrap_or_else(|| payload.get("diagnostic").cloned().unwrap_or(Value::Null)),
            "adapter_contract": adapter_contract_context(),
        }),
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
            "decision": normalize_permission_decision(payload.get("decision").and_then(Value::as_str).unwrap_or("denied")),
            "reason": payload.get("reason").cloned().unwrap_or(Value::Null),
            "adapter_contract": adapter_contract_context(),
        }),
    )
    .await
}

fn normalize_permission_decision(decision: &str) -> &'static str {
    match decision {
        "approve" | "approved" | "allow" | "allow_once" | "allow_url" | "allow_host"
        | "granted" => "granted",
        "timeout" | "timed_out" => "expired",
        _ => "denied",
    }
}

pub(crate) async fn try_handle_prompt(
    http: &reqwest::Client,
    config: &Config,
    adapter_state: &mut AdapterState,
    shared_state: &AdapterSharedState,
    response_id: Value,
    session_id: &str,
    prompt: &str,
    client_context: Value,
    conversation_id: Option<&str>,
    requested_mode: &str,
    turn_token: Uuid,
) -> Result<bool> {
    if !enabled() {
        return Ok(false);
    }
    match handle_prompt(
        http,
        config,
        adapter_state,
        shared_state,
        response_id,
        session_id,
        prompt,
        client_context,
        conversation_id,
        requested_mode,
        turn_token,
    )
    .await
    {
        Ok(()) => Ok(true),
        Err(err) if required() => Err(err),
        Err(err) => {
            eprintln!(
                "bear-armature: BearWire prompt failed; falling back to legacy ACP HTTP session_id={} error={err:#}",
                session_id
            );
            Ok(false)
        }
    }
}

async fn rpc_call(
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
    let tool_call_id = data
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let tool_name = data
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let approval_request_id = data
        .get("approval_request_id")
        .and_then(Value::as_str)
        .or_else(|| resource_ref_id(event, "permission_request"));
    json!({
        "type": "tool_request",
        "run_id": event.get("run_id").and_then(Value::as_str),
        "tool_call_id": tool_call_id,
        "tool_name": tool_name,
        "title": data.get("title").cloned().unwrap_or(Value::Null),
        "args": data.get("arguments").cloned().unwrap_or_else(|| json!({})),
        "approval_request_id": approval_request_id,
        "approval": {
            "required": approval_required,
            "reason": data.get("reason").cloned().unwrap_or(Value::Null),
        },
    })
}

fn bearwire_permission_event_to_legacy_permission_request(event: &Value) -> Value {
    let data = event.get("data").unwrap_or(&Value::Null);
    let permission_id = data
        .get("approval_request_id")
        .or_else(|| data.get("permission_id"))
        .and_then(Value::as_str)
        .or_else(|| resource_ref_id(event, "permission_request"))
        .unwrap_or("unknown");
    let tool_call_id = data
        .get("tool_call_id")
        .and_then(Value::as_str)
        .or_else(|| resource_ref_id(event, "tool_call"))
        .unwrap_or(permission_id);
    let tool_name = data
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    json!({
        "type": "permission_request",
        "run_id": event.get("run_id").and_then(Value::as_str),
        "permission_id": permission_id,
        "tool_call_id": tool_call_id,
        "tool_name": tool_name,
        "title": data.get("title").and_then(Value::as_str).unwrap_or("Permission request"),
        "reason": data.get("reason").and_then(Value::as_str).unwrap_or("BEARS requests permission."),
        "target": data.get("arguments").cloned().unwrap_or_else(|| json!({ "kind": "tool_call" })),
    })
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
        "run.progress" => {
            let data = event.get("data").unwrap_or(&Value::Null);
            let text = data.get("text").and_then(Value::as_str).unwrap_or("");
            let kind = data
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("progress");
            let elapsed_ms = data.get("elapsed_ms").and_then(Value::as_u64);
            // Progress is observability, not model-visible output. Do not let it satisfy
            // prompt completion checks or suppress first-assistant/tool-event diagnostics.
            eprintln!(
                "bear-armature: BearWire progress session_id={} run_id={} kind={} elapsed_ms={} text={}",
                session_id,
                event.get("run_id").and_then(Value::as_str).unwrap_or("<unknown>"),
                kind,
                elapsed_ms.map(|value| value.to_string()).unwrap_or_else(|| "unknown".to_string()),
                if text.is_empty() { "<empty>" } else { text }
            );
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
            outcome.saw_done = true;
            outcome.saw_error = true;
            let message = event
                .pointer("/data/message")
                .and_then(Value::as_str)
                .unwrap_or("BearWire run failed");
            let legacy = json!({
                "type": "error",
                "message": message,
                "detail": event.get("data").cloned().unwrap_or(Value::Null),
                "terminal": { "outcome": "failed" }
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
        "run.cancelled" => {
            outcome.saw_done = true;
            outcome.saw_error = true;
            let legacy = json!({
                "type": "error",
                "message": "BEARS request was cancelled.",
                "terminal": { "outcome": "cancelled", "recovery_hint": "none" }
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
        "tool_call.blocked" => {
            outcome.saw_tool_activity = true;
            outcome.saw_visible_output = true;
            diagnostics.saw_tool_activity = true;
            diagnostics.saw_visible_output = true;
            let legacy = bearwire_tool_event_to_legacy_tool_request(event, true);
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
        "run.paused" => {
            let reason = event
                .pointer("/data/reason")
                .and_then(Value::as_str)
                .unwrap_or("paused");
            let resume_token = event
                .pointer("/data/resume_token")
                .and_then(Value::as_str)
                .unwrap_or("<none>");
            outcome.saw_tool_activity = reason == "requires_approval";
            diagnostics.saw_tool_activity |= outcome.saw_tool_activity;
            eprintln!(
                "bear-armature: BearWire run paused session_id={} reason={} resume_token={}",
                session_id, reason, resume_token
            );
        }
        "permission.requested" => {
            outcome.saw_tool_activity = true;
            outcome.saw_visible_output = true;
            diagnostics.saw_tool_activity = true;
            diagnostics.saw_visible_output = true;
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
