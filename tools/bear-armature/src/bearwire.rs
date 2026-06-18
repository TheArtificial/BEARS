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

pub(crate) fn enabled() -> bool {
    env_bool("BEARS_BEARWIRE")
}

pub(crate) fn required() -> bool {
    env_bool("BEARS_BEARWIRE_REQUIRED")
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
            "client_context": client_context,
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

    while started.elapsed() < BEARWIRE_PROMPT_TIMEOUT {
        let replay = fetch_events(http, config, session_id, after).await?;
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
            let text = event
                .get("data")
                .and_then(|data| data.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            outcome.saw_visible_output = !text.is_empty();
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
        // Tool/permission continuation is the next Phase 3 slice. Treat these as visible
        // activity for now so a tool request does not look like an empty turn, but do not
        // invent continuation semantics until client.* is wired end-to-end.
        "tool_call.requested" | "tool_call.blocked" | "permission.requested" => {
            outcome.saw_tool_activity = true;
            outcome.saw_visible_output = true;
            diagnostics.saw_tool_activity = true;
            diagnostics.saw_visible_output = true;
            eprintln!(
                "bear-armature: BearWire event type {} received but tool/permission continuation is not wired yet event={}",
                ty,
                truncate_for_log(&event.to_string(), 400)
            );
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
