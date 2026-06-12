//! Wraps the Codepool `reqwest` byte stream with TTFB logging and terminal outcome metrics.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use bytes::Bytes;
use futures::ready;
use futures::{Future, Stream};
use sqlx::PgPool;
use uuid::Uuid;

use crate::core::conversation_persistence::{append_message, ensure_conversation_for_external_id};

use super::metrics;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Terminal {
    Ok,
    Empty,
    ProxyError,
}

/// Streams bytes from Codepool to the browser while recording observability.
pub struct ChatSseProxyStream {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    request_id: Uuid,
    user_id: i32,
    bear_id: Uuid,
    conversation_id: String,
    started_at: Instant,
    first_byte_at: Option<Instant>,
    total_bytes: usize,
    terminal: Option<Terminal>,
}

impl ChatSseProxyStream {
    pub fn new(
        inner: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
        request_id: Uuid,
        user_id: i32,
        bear_id: Uuid,
        conversation_id: String,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
            request_id,
            user_id,
            bear_id,
            conversation_id,
            started_at: Instant::now(),
            first_byte_at: None,
            total_bytes: 0,
            terminal: None,
        }
    }

    fn record_terminal(&mut self, t: Terminal) {
        if self.terminal.is_some() {
            return;
        }
        self.terminal = Some(t);
        match t {
            Terminal::Ok => metrics::chat_send_finished_ok(),
            Terminal::Empty => metrics::chat_send_finished_empty(),
            Terminal::ProxyError => metrics::chat_send_finished_proxy_error(),
        }
    }

    fn log_and_record_finish(&mut self, t: Terminal) {
        self.record_terminal(t);
        let elapsed = self.started_at.elapsed();
        let ttfb_ms = self.first_byte_at.map(|fb| {
            fb.duration_since(self.started_at)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64
        });
        match t {
            Terminal::Ok => {
                tracing::info!(
                    request_id = %self.request_id,
                    user_id = %self.user_id,
                    bear_id = %self.bear_id,
                    conversation_id = %self.conversation_id,
                    total_bytes = self.total_bytes,
                    ttfb_ms,
                    elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
                    "chat_send sse stream finished ok"
                );
            }
            Terminal::Empty => {
                tracing::warn!(
                    request_id = %self.request_id,
                    user_id = %self.user_id,
                    bear_id = %self.bear_id,
                    conversation_id = %self.conversation_id,
                    elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
                    "chat_send sse stream ended with zero bytes from upstream"
                );
            }
            Terminal::ProxyError => {}
        }
    }
}

impl Drop for ChatSseProxyStream {
    fn drop(&mut self) {
        if self.terminal.is_some() {
            return;
        }
        metrics::chat_send_finished_proxy_error();
        tracing::warn!(
            request_id = %self.request_id,
            user_id = %self.user_id,
            bear_id = %self.bear_id,
            conversation_id = %self.conversation_id,
            total_bytes = self.total_bytes,
            "chat_send sse proxy stream dropped before terminal poll (client disconnect or task cancelled)"
        );
    }
}

fn rich_event_status_text(event: &serde_json::Value) -> Option<String> {
    let ty = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let tool = event.get("tool").and_then(|v| v.as_str()).unwrap_or("tool");
    let name = event
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("sub-agent");
    let summary = event.get("summary").and_then(|v| v.as_str()).unwrap_or("");
    let text = match ty {
        "server_tool_started" => format!("Started {tool}"),
        "server_tool_finished" => {
            if summary.is_empty() {
                format!("Finished {tool}")
            } else {
                format!("Finished {tool}: {summary}")
            }
        }
        "subagent_started" => format!("Started sub-agent {name}"),
        "subagent_finished" => {
            if summary.is_empty() {
                format!("Finished sub-agent {name}")
            } else {
                format!("Finished sub-agent {name}: {summary}")
            }
        }
        "memory_update_recorded" => {
            if summary.is_empty() {
                "Recorded memory update".to_string()
            } else {
                format!("Recorded memory update: {summary}")
            }
        }
        _ => return None,
    };
    Some(text)
}

#[derive(Default)]
struct PendingConversationPersistence {
    assistant_text: String,
    /// User-visible status lines (tool started/finished, progress) shown during the live stream.
    status_text: String,
    error_text: String,
    reasoning_text: String,
    resolved_conversation_id: Option<String>,
    workflow_events: Vec<serde_json::Value>,
}

impl PendingConversationPersistence {
    fn ingest(&mut self, event: &serde_json::Value) {
        match event.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "assistant_delta" => {
                if let Some(text) = event.get("text").and_then(|v| v.as_str()) {
                    self.assistant_text.push_str(text);
                }
            }
            "reasoning_delta" => {
                if let Some(text) = event.get("text").and_then(|v| v.as_str()) {
                    self.reasoning_text.push_str(text);
                }
            }
            "error" => {
                if let Some(message) = event.get("message").and_then(|v| v.as_str()) {
                    if !self.error_text.is_empty() {
                        self.error_text.push_str("\n\n");
                    }
                    self.error_text.push_str(message);
                }
            }
            "server_tool_started"
            | "server_tool_finished"
            | "subagent_started"
            | "subagent_finished"
            | "memory_update_recorded" => {
                if let Some(text) = rich_event_status_text(event) {
                    if !self.status_text.is_empty() {
                        self.status_text.push('\n');
                    }
                    self.status_text.push_str(&text);
                }
            }
            "conversation_resolved" => {
                if let Some(conversation_id) = event.get("conversation_id").and_then(|v| v.as_str()) {
                    self.resolved_conversation_id = Some(conversation_id.to_string());
                }
                self.workflow_events.push(event.clone());
            }
            _ => {}
        }
    }

    fn has_flushable_content(&self) -> bool {
        !self.assistant_text.trim().is_empty()
            || !self.status_text.trim().is_empty()
            || !self.error_text.trim().is_empty()
            || !self.reasoning_text.trim().is_empty()
            || !self.workflow_events.is_empty()
    }

    async fn flush(
        self,
        pool: &PgPool,
        bear_id: Uuid,
        user_id: i32,
        conversation_id: &str,
        request_id: Uuid,
        interrupted: bool,
    ) {
        let external_conversation_id = self
            .resolved_conversation_id
            .as_deref()
            .unwrap_or(conversation_id);
        let canonical = match ensure_conversation_for_external_id(
            pool,
            bear_id,
            Some(user_id),
            external_conversation_id,
            None,
            None,
        )
        .await
        {
            Ok(conversation) => conversation,
            Err(err) => {
                tracing::warn!(bear_id = %bear_id, conversation_id = conversation_id, resolved_conversation_id = external_conversation_id, error = %err, "failed to ensure canonical web chat conversation during flush");
                return;
            }
        };

        if !self.reasoning_text.trim().is_empty() {
            if let Err(err) = append_message(
                pool,
                canonical.id,
                &crate::core::conversation_message_types::ConversationMessageWrite::assistant_turn(
                    &self.reasoning_text,
                    serde_json::json!({
                        "type": "reasoning_delta_coalesced",
                        "text": self.reasoning_text,
                        "request_id": request_id.to_string(),
                        "interrupted": interrupted,
                    }),
                )
                .with_source_event_id(Some(format!("web-chat-reasoning:{request_id}"))),
            )
            .await
            {
                tracing::warn!(bear_id = %bear_id, conversation_id = conversation_id, resolved_conversation_id = external_conversation_id, error = %err, "failed to append canonical web chat reasoning");
            }
        }

        if !self.status_text.trim().is_empty() {
            if let Err(err) = append_message(
                pool,
                canonical.id,
                &crate::core::conversation_message_types::ConversationMessageWrite::assistant_turn(
                    &self.status_text,
                    serde_json::json!({
                        "type": "status_message_coalesced",
                        "text": self.status_text,
                        "request_id": request_id.to_string(),
                        "interrupted": interrupted,
                    }),
                )
                .with_source_event_id(Some(format!("web-chat-status:{request_id}"))),
            )
            .await
            {
                tracing::warn!(bear_id = %bear_id, conversation_id = conversation_id, resolved_conversation_id = external_conversation_id, error = %err, "failed to append canonical web chat status");
            }
        }

        if !self.assistant_text.trim().is_empty() {
            if let Err(err) = append_message(
                pool,
                canonical.id,
                &crate::core::conversation_message_types::ConversationMessageWrite::assistant_turn(
                    &self.assistant_text,
                    serde_json::json!({
                        "type": "assistant_delta_coalesced",
                        "text": self.assistant_text,
                        "request_id": request_id.to_string(),
                        "interrupted": interrupted,
                    }),
                )
                .with_source_event_id(Some(format!("web-chat-assistant:{request_id}"))),
            )
            .await
            {
                tracing::warn!(bear_id = %bear_id, conversation_id = conversation_id, resolved_conversation_id = external_conversation_id, error = %err, "failed to append canonical web chat assistant output");
            }
        }

        if !self.error_text.trim().is_empty() {
            if let Err(err) = append_message(
                pool,
                canonical.id,
                &crate::core::conversation_message_types::ConversationMessageWrite::assistant_turn(
                    &self.error_text,
                    serde_json::json!({
                        "type": "error_message_coalesced",
                        "text": self.error_text,
                        "request_id": request_id.to_string(),
                        "interrupted": interrupted,
                    }),
                )
                .with_source_event_id(Some(format!("web-chat-error:{request_id}"))),
            )
            .await
            {
                tracing::warn!(bear_id = %bear_id, conversation_id = conversation_id, resolved_conversation_id = external_conversation_id, error = %err, "failed to append canonical web chat error");
            }
        }

        for event in self.workflow_events {
            if let Err(err) = append_message(
                pool,
                canonical.id,
                &crate::core::conversation_message_types::ConversationMessageWrite::workflow_diagnostic(
                    "Conversation resolved",
                    event,
                ),
            )
            .await
            {
                tracing::warn!(bear_id = %bear_id, conversation_id = conversation_id, resolved_conversation_id = external_conversation_id, error = %err, "failed to append canonical web chat workflow event");
            }
        }
    }
}

pub(crate) fn bear_channel_sse_bytes(event: &serde_json::Value) -> Option<Bytes> {
    Some(Bytes::from(format!("data: {}\n\n", event)))
}

pub(crate) fn sse_comment_keepalive_bytes() -> Bytes {
    Bytes::from(": keepalive\n\n")
}

fn empty_terminal_bear_channel_error(request_id: Uuid) -> serde_json::Value {
    serde_json::json!({
        "type": "error",
        "message": "The assistant stream ended without a reply.",
        "detail": "No displayable content reached the browser before the stream closed. You can retry, or share the reference below when asking for help.",
        "error_type": "stream_empty_terminal",
        "request_id": request_id.to_string(),
    })
}

fn browser_empty_terminal_error(request_id: Uuid) -> Bytes {
    let mapped = serde_json::json!({
        "message_type": "error_message",
        "message": "The assistant stream ended without a reply.",
        "detail": "No displayable content reached the browser before the stream closed. You can retry, or share the reference below when asking for help.",
        "error_type": "stream_empty_terminal",
        "support_ref": request_id.to_string(),
    });
    bear_channel_sse_bytes(&mapped).expect("empty terminal error serializes")
}

fn bear_channel_event_to_deep_chat_sse(event: &serde_json::Value) -> Option<Bytes> {
    let ty = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let mapped = match ty {
        "assistant_delta" => serde_json::json!({
            "message_type": "assistant_message",
            "content": event.get("text").and_then(|v| v.as_str()).unwrap_or(""),
            "id": event.get("id").and_then(|v| v.as_str()),
        }),
        "reasoning_delta" => {
            let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
            serde_json::json!({
                "message_type": "reasoning_message",
                "reasoning": text,
                "content": text,
                "id": event.get("id").and_then(|v| v.as_str()),
            })
        }
        "error" => serde_json::json!({
            "message_type": "error_message",
            "message": event.get("message").and_then(|v| v.as_str()).unwrap_or("Upstream error"),
            "detail": event.get("detail").and_then(|v| v.as_str()),
            "error_type": event.get("error_type").and_then(|v| v.as_str()),
            "support_ref": event.get("request_id").and_then(|v| v.as_str()),
            "context": event.get("context"),
        }),
        "conversation_resolved" => serde_json::json!({
            "message_type": "conversation_resolved",
            "conversation_id": event.get("conversation_id").and_then(|v| v.as_str()),
        }),
        // `done` is terminal control metadata, not user-visible status.
        "done" => return None,
        "server_tool_started"
        | "server_tool_finished"
        | "subagent_started"
        | "subagent_finished"
        | "memory_update_recorded" => {
            let text = rich_event_status_text(event)?;
            serde_json::json!({
                "message_type": "status_message",
                "content": text,
                "status_type": ty,
            })
        }
        _ => return None,
    };
    bear_channel_sse_bytes(&mapped)
}

pub(crate) fn map_bear_channel_sse_frame(frame: &[u8]) -> Vec<Bytes> {
    let text = String::from_utf8_lossy(frame);
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
            if let Some(bytes) = bear_channel_event_to_deep_chat_sse(&value) {
                out.push(bytes);
            }
        }
    }
    out
}

/// Streams `bear_channel` SSE from Codepool to the browser after translating channel events
/// into the existing Deep Chat / Letta-shaped SSE payloads consumed by `bear_chat.html`.
pub struct BearChannelSseProxyStream {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    request_id: Uuid,
    user_id: i32,
    bear_id: Uuid,
    conversation_id: String,
    pool: PgPool,
    started_at: Instant,
    first_byte_at: Option<Instant>,
    total_bytes: usize,
    terminal: Option<Terminal>,
    buffer: Vec<u8>,
    pending: VecDeque<Bytes>,
    persistence: PendingConversationPersistence,
    flush_started: bool,
    flush_task: Option<tokio::task::JoinHandle<()>>,
    empty_terminal_error_emitted: bool,
}

impl BearChannelSseProxyStream {
    pub fn new(
        inner: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
        request_id: Uuid,
        user_id: i32,
        bear_id: Uuid,
        conversation_id: String,
        pool: PgPool,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
            request_id,
            user_id,
            bear_id,
            conversation_id,
            pool,
            started_at: Instant::now(),
            first_byte_at: None,
            total_bytes: 0,
            terminal: None,
            buffer: Vec::new(),
            pending: VecDeque::new(),
            persistence: PendingConversationPersistence::default(),
            flush_started: false,
            flush_task: None,
            empty_terminal_error_emitted: false,
        }
    }

    fn queue_empty_terminal_error_if_needed(&mut self) -> bool {
        if self.total_bytes > 0 || self.empty_terminal_error_emitted {
            return false;
        }
        self.empty_terminal_error_emitted = true;
        let error_event = empty_terminal_bear_channel_error(self.request_id);
        self.persistence.ingest(&error_event);
        self.pending
            .push_back(browser_empty_terminal_error(self.request_id));
        true
    }

    /// Persist browser-visible stream content. Runs on normal stream end and on abrupt drop.
    fn schedule_persistence_flush(&mut self, interrupted: bool) {
        if self.flush_started || !self.persistence.has_flushable_content() {
            return;
        }
        self.flush_started = true;
        let persistence = std::mem::take(&mut self.persistence);
        let pool = self.pool.clone();
        let bear_id = self.bear_id;
        let user_id = self.user_id;
        let conversation_id = self.conversation_id.clone();
        let request_id = self.request_id;
        self.flush_task = Some(tokio::spawn(async move {
            persistence
                .flush(
                    &pool,
                    bear_id,
                    user_id,
                    &conversation_id,
                    request_id,
                    interrupted,
                )
                .await;
        }));
    }

    fn record_terminal(&mut self, t: Terminal) {
        if self.terminal.is_some() {
            return;
        }
        self.terminal = Some(t);
        match t {
            Terminal::Ok => metrics::chat_send_finished_ok(),
            Terminal::Empty => metrics::chat_send_finished_empty(),
            Terminal::ProxyError => metrics::chat_send_finished_proxy_error(),
        }
    }

    fn log_and_record_finish(&mut self, t: Terminal) {
        self.record_terminal(t);
        let elapsed = self.started_at.elapsed();
        let ttfb_ms = self.first_byte_at.map(|fb| {
            fb.duration_since(self.started_at)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64
        });
        match t {
            Terminal::Ok => tracing::info!(
                request_id = %self.request_id,
                user_id = %self.user_id,
                bear_id = %self.bear_id,
                conversation_id = %self.conversation_id,
                total_bytes = self.total_bytes,
                ttfb_ms,
                elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
                "chat_send bear_channel stream finished ok"
            ),
            Terminal::Empty => tracing::warn!(
                request_id = %self.request_id,
                user_id = %self.user_id,
                bear_id = %self.bear_id,
                conversation_id = %self.conversation_id,
                browser_bytes = self.total_bytes,
                empty_terminal_error_emitted = self.empty_terminal_error_emitted,
                elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
                "chat_send bear_channel stream ended with zero browser-compatible bytes"
            ),
            Terminal::ProxyError => {}
        }
    }

}

impl Drop for BearChannelSseProxyStream {
    fn drop(&mut self) {
        if self.terminal.is_none() {
            self.schedule_persistence_flush(true);
        }
        if self.terminal.is_some() {
            return;
        }
        metrics::chat_send_finished_proxy_error();
        metrics::record_chat_send_dropped(self.total_bytes > 0);
        let elapsed_ms = self.started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let ttfb_ms = self.first_byte_at.map(|fb| {
            fb.duration_since(self.started_at)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64
        });
        tracing::warn!(
            request_id = %self.request_id,
            user_id = %self.user_id,
            bear_id = %self.bear_id,
            conversation_id = %self.conversation_id,
            browser_bytes = self.total_bytes,
            ttfb_ms,
            elapsed_ms,
            empty_terminal_error_emitted = self.empty_terminal_error_emitted,
            flush_scheduled = self.flush_started,
            "chat_send bear_channel proxy stream dropped before terminal poll (client disconnect or task cancelled)"
        );
    }
}

impl Stream for BearChannelSseProxyStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        if let Some(bytes) = this.pending.pop_front() {
            if this.total_bytes == 0 {
                let ttfb_ms = this
                    .started_at
                    .elapsed()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64;
                metrics::record_chat_send_ttfb_ms(ttfb_ms);
                if this.first_byte_at.is_none() {
                    this.first_byte_at = Some(Instant::now());
                    tracing::info!(
                        request_id = %this.request_id,
                        user_id = %this.user_id,
                        bear_id = %this.bear_id,
                        conversation_id = %this.conversation_id,
                        ttfb_ms,
                        "chat_send first browser-visible byte"
                    );
                }
            }
            this.total_bytes += bytes.len();
            return Poll::Ready(Some(Ok(bytes)));
        }

        loop {
            match ready!(this.inner.as_mut().poll_next(cx)) {
                Some(Ok(chunk)) => {
                    if this.first_byte_at.is_none() {
                        this.first_byte_at = Some(Instant::now());
                        let ttfb = this.started_at.elapsed();
                        tracing::info!(
                            request_id = %this.request_id,
                            user_id = %this.user_id,
                            bear_id = %this.bear_id,
                            conversation_id = %this.conversation_id,
                            ttfb_ms = ttfb.as_millis().min(u128::from(u64::MAX)) as u64,
                            "chat_send first bear_channel byte from Codepool"
                        );
                    }
                    this.buffer.extend_from_slice(&chunk);
                    while let Some(pos) = this.buffer.windows(2).position(|w| w == b"\n\n") {
                        let frame: Vec<u8> = this.buffer.drain(..pos + 2).collect();
                        let text = String::from_utf8_lossy(&frame);
                        if text
                            .lines()
                            .all(|line| line.is_empty() || line.starts_with(':'))
                        {
                            this.pending.push_back(Bytes::from(frame));
                            continue;
                        }
                        for line in text.lines() {
                            let Some(data) = line.strip_prefix("data:") else {
                                continue;
                            };
                            let data = data.trim();
                            if data.is_empty() || data == "[DONE]" {
                                continue;
                            }
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                                this.persistence.ingest(&value);
                                if let Some(bytes) = bear_channel_event_to_deep_chat_sse(&value) {
                                    this.pending.push_back(bytes);
                                }
                            }
                        }
                    }
                    if let Some(bytes) = this.pending.pop_front() {
                        this.total_bytes += bytes.len();
                        return Poll::Ready(Some(Ok(bytes)));
                    }
                }
                Some(Err(e)) => {
                    this.schedule_persistence_flush(true);
                    this.log_and_record_finish(Terminal::ProxyError);
                    tracing::error!(
                        request_id = %this.request_id,
                        user_id = %this.user_id,
                        bear_id = %this.bear_id,
                        conversation_id = %this.conversation_id,
                        error = %e,
                        total_bytes = this.total_bytes,
                        "chat_send bear_channel proxy chunk error from Codepool"
                    );
                    return Poll::Ready(Some(Err(std::io::Error::other(e.to_string()))));
                }
                None => {
                    if !this.buffer.is_empty() {
                        let frame = std::mem::take(&mut this.buffer);
                        let text = String::from_utf8_lossy(&frame);
                        for line in text.lines() {
                            let Some(data) = line.strip_prefix("data:") else {
                                continue;
                            };
                            let data = data.trim();
                            if data.is_empty() || data == "[DONE]" {
                                continue;
                            }
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                                this.persistence.ingest(&value);
                                if let Some(bytes) = bear_channel_event_to_deep_chat_sse(&value) {
                                    this.pending.push_back(bytes);
                                }
                            }
                        }
                        if let Some(bytes) = this.pending.pop_front() {
                            this.total_bytes += bytes.len();
                            return Poll::Ready(Some(Ok(bytes)));
                        }
                    }
                    if this.terminal.is_some() {
                        return Poll::Ready(None);
                    }
                    if this.queue_empty_terminal_error_if_needed() {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    if !this.pending.is_empty() {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    if !this.flush_started {
                        this.schedule_persistence_flush(false);
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    if let Some(task) = this.flush_task.as_mut() {
                        match Pin::new(task).poll(cx) {
                            Poll::Pending => return Poll::Pending,
                            Poll::Ready(Ok(())) | Poll::Ready(Err(_)) => {
                                this.flush_task = None;
                            }
                        }
                    }
                    let outcome = if this.empty_terminal_error_emitted || this.total_bytes == 0 {
                        Terminal::Empty
                    } else {
                        Terminal::Ok
                    };
                    this.log_and_record_finish(outcome);
                    return Poll::Ready(None);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped_text(frame: &str) -> String {
        map_bear_channel_sse_frame(frame.as_bytes())
            .into_iter()
            .map(|b| String::from_utf8(b.to_vec()).expect("utf8"))
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn maps_assistant_delta_to_deep_chat_sse() {
        let out =
            mapped_text("data: {\"type\":\"assistant_delta\",\"text\":\"Hi\",\"id\":\"a1\"}\n\n");
        assert!(out.starts_with("data: "));
        assert!(out.contains("\"message_type\":\"assistant_message\""));
        assert!(out.contains("\"content\":\"Hi\""));
    }

    #[test]
    fn maps_reasoning_delta_to_deep_chat_sse() {
        let out = mapped_text("data: {\"type\":\"reasoning_delta\",\"text\":\"Thinking\"}\n\n");
        assert!(out.contains("\"message_type\":\"reasoning_message\""));
        assert!(out.contains("\"reasoning\":\"Thinking\""));
    }

    #[test]
    fn persistence_ingest_tracks_browser_visible_content() {
        let mut persistence = PendingConversationPersistence::default();
        assert!(!persistence.has_flushable_content());
        persistence.ingest(&serde_json::json!({"type": "assistant_delta", "text": "Hello"}));
        assert!(persistence.has_flushable_content());
        persistence.ingest(&serde_json::json!({"type": "error", "message": "Tool failed"}));
        persistence.ingest(&serde_json::json!({
            "type": "server_tool_started",
            "tool": "memory_read",
            "summary": "memory_read"
        }));
        persistence.ingest(&serde_json::json!({
            "type": "subagent_finished",
            "name": "researcher",
            "summary": "done"
        }));
        assert!(!persistence.assistant_text.is_empty());
        assert!(!persistence.error_text.is_empty());
        assert!(!persistence.status_text.is_empty());
    }

    #[test]
    fn maps_error_to_deep_chat_sse() {
        let out = mapped_text("data: {\"type\":\"error\",\"message\":\"Nope\",\"detail\":\"More\",\"request_id\":\"r1\",\"context\":{\"upstream_error\":[{\"param\":\"tools[15].name\"}]}}\n\n");
        assert!(out.contains("\"message_type\":\"error_message\""));
        assert!(out.contains("\"message\":\"Nope\""));
        assert!(out.contains("\"support_ref\":\"r1\""));
        assert!(out.contains("\"upstream_error\""));
    }

    #[test]
    fn maps_rich_events_to_status_messages() {
        let out =
            mapped_text("data: {\"type\":\"server_tool_started\",\"tool\":\"cabinet.search\"}\n\n");
        assert!(out.contains("\"message_type\":\"status_message\""));
        assert!(out.contains("Started cabinet.search"));
    }

    #[test]
    fn drops_done_control_event() {
        let out = mapped_text("data: {\"type\":\"done\",\"outcome\":\"ok\"}\n\n");
        assert!(out.is_empty());
    }

    #[test]
    fn empty_terminal_error_includes_support_ref() {
        let bytes = browser_empty_terminal_error(Uuid::parse_str("f42114ea-99bd-48a7-818a-78d4e3d914be").unwrap());
        let text = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(text.contains("stream_empty_terminal") || text.contains("error_message"));
        assert!(text.contains("f42114ea-99bd-48a7-818a-78d4e3d914be"));
    }

    #[test]
    fn sse_comment_keepalive_bytes_format() {
        let bytes = sse_comment_keepalive_bytes();
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), ": keepalive\n\n");
    }
}

impl Stream for ChatSseProxyStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        match ready!(this.inner.as_mut().poll_next(cx)) {
            Some(Ok(chunk)) => {
                if this.first_byte_at.is_none() {
                    this.first_byte_at = Some(Instant::now());
                    let ttfb = this.started_at.elapsed();
                    tracing::info!(
                        request_id = %this.request_id,
                        user_id = %this.user_id,
                        bear_id = %this.bear_id,
                        conversation_id = %this.conversation_id,
                        ttfb_ms = ttfb.as_millis().min(u128::from(u64::MAX)) as u64,
                        "chat_send first byte from Codepool"
                    );
                }
                this.total_bytes += chunk.len();
                Poll::Ready(Some(Ok(chunk)))
            }
            Some(Err(e)) => {
                this.log_and_record_finish(Terminal::ProxyError);
                tracing::error!(
                    request_id = %this.request_id,
                    user_id = %this.user_id,
                    bear_id = %this.bear_id,
                    conversation_id = %this.conversation_id,
                    error = %e,
                    total_bytes = this.total_bytes,
                    "chat_send sse proxy chunk error from Codepool"
                );
                Poll::Ready(Some(Err(std::io::Error::other(e.to_string()))))
            }
            None => {
                if this.terminal.is_some() {
                    // Inner may emit Err then None; terminal already recorded.
                    return Poll::Ready(None);
                }
                let outcome = if this.total_bytes == 0 {
                    Terminal::Empty
                } else {
                    Terminal::Ok
                };
                this.log_and_record_finish(outcome);
                Poll::Ready(None)
            }
        }
    }
}
