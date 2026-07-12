//! In-memory counters exposed as Prometheus text (no external metrics crates).
use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static CHAT_SEND_STARTED: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_FINISHED_OK: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_FINISHED_EMPTY: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_FINISHED_PROXY_ERROR: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_RUNTIME_LEGACY: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_RUNTIME_BEAR_CHANNEL: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_RUNTIME_NATIVE: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_TTFB_MS_SUM: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_TTFB_MS_COUNT: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_DROPPED_TOTAL: AtomicU64 = AtomicU64::new(0);
static CHAT_SEND_DROPPED_WITH_BYTES_TOTAL: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn chat_send_started() {
    CHAT_SEND_STARTED.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn chat_send_finished_ok() {
    CHAT_SEND_FINISHED_OK.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn chat_send_finished_empty() {
    CHAT_SEND_FINISHED_EMPTY.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn chat_send_finished_proxy_error() {
    CHAT_SEND_FINISHED_PROXY_ERROR.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn chat_send_runtime_legacy() {
    CHAT_SEND_RUNTIME_LEGACY.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn chat_send_runtime_bear_channel() {
    CHAT_SEND_RUNTIME_BEAR_CHANNEL.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn chat_send_runtime_native() {
    CHAT_SEND_RUNTIME_NATIVE.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn record_chat_send_ttfb_ms(ms: u64) {
    CHAT_SEND_TTFB_MS_SUM.fetch_add(ms, Ordering::Relaxed);
    CHAT_SEND_TTFB_MS_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn record_chat_send_dropped(had_browser_bytes: bool) {
    CHAT_SEND_DROPPED_TOTAL.fetch_add(1, Ordering::Relaxed);
    if had_browser_bytes {
        CHAT_SEND_DROPPED_WITH_BYTES_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
}

/// Prometheus text exposition 0.0.4.
pub fn render_prometheus_text() -> String {
    let mut s = String::with_capacity(768);
    let a = CHAT_SEND_STARTED.load(Ordering::Relaxed);
    let b = CHAT_SEND_FINISHED_OK.load(Ordering::Relaxed);
    let c = CHAT_SEND_FINISHED_EMPTY.load(Ordering::Relaxed);
    let d = CHAT_SEND_FINISHED_PROXY_ERROR.load(Ordering::Relaxed);
    let e = CHAT_SEND_RUNTIME_LEGACY.load(Ordering::Relaxed);
    let f = CHAT_SEND_RUNTIME_BEAR_CHANNEL.load(Ordering::Relaxed);
    let g = CHAT_SEND_RUNTIME_NATIVE.load(Ordering::Relaxed);
    let ttfb_sum = CHAT_SEND_TTFB_MS_SUM.load(Ordering::Relaxed);
    let ttfb_count = CHAT_SEND_TTFB_MS_COUNT.load(Ordering::Relaxed);
    let dropped = CHAT_SEND_DROPPED_TOTAL.load(Ordering::Relaxed);
    let dropped_with_bytes = CHAT_SEND_DROPPED_WITH_BYTES_TOTAL.load(Ordering::Relaxed);

    writeln!(
        s,
        "# HELP den_chat_send_started_total Chat POST /v1/chat/send requests that passed auth and started an SSE response."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_started_total counter").unwrap();
    writeln!(s, "den_chat_send_started_total {a}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_finished_ok_total SSE streams that forwarded at least one byte."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_finished_ok_total counter").unwrap();
    writeln!(s, "den_chat_send_finished_ok_total {b}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_finished_empty_upstream_total SSE streams that ended with zero bytes from upstream."
    )
    .unwrap();
    writeln!(
        s,
        "# TYPE den_chat_send_finished_empty_upstream_total counter"
    )
    .unwrap();
    writeln!(s, "den_chat_send_finished_empty_upstream_total {c}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_finished_proxy_error_total SSE proxy failures (chunk error or drop before completion)."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_finished_proxy_error_total counter").unwrap();
    writeln!(s, "den_chat_send_finished_proxy_error_total {d}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_runtime_legacy_total Chat send requests routed through a legacy conversation endpoint."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_runtime_legacy_total counter").unwrap();
    writeln!(s, "den_chat_send_runtime_legacy_total {e}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_runtime_bear_channel_total Chat send requests routed through a legacy bear_channel path."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_runtime_bear_channel_total counter").unwrap();
    writeln!(s, "den_chat_send_runtime_bear_channel_total {f}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_runtime_native_total Chat send requests routed through the Den-native in-process loop."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_runtime_native_total counter").unwrap();
    writeln!(s, "den_chat_send_runtime_native_total {g}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_ttfb_ms_sum Sum of browser-visible time-to-first-byte for chat SSE streams (milliseconds)."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_ttfb_ms_sum counter").unwrap();
    writeln!(s, "den_chat_send_ttfb_ms_sum {ttfb_sum}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_ttfb_ms_count Count of chat SSE streams with a recorded browser-visible TTFB."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_ttfb_ms_count counter").unwrap();
    writeln!(s, "den_chat_send_ttfb_ms_count {ttfb_count}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_dropped_total Chat SSE proxy streams dropped before a terminal poll (client disconnect or task cancelled)."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_dropped_total counter").unwrap();
    writeln!(s, "den_chat_send_dropped_total {dropped}").unwrap();

    writeln!(
        s,
        "# HELP den_chat_send_dropped_with_bytes_total Chat SSE proxy drops that had already sent at least one browser-visible byte."
    )
    .unwrap();
    writeln!(s, "# TYPE den_chat_send_dropped_with_bytes_total counter").unwrap();
    writeln!(
        s,
        "den_chat_send_dropped_with_bytes_total {dropped_with_bytes}"
    )
    .unwrap();

    s
}

#[cfg(test)]
mod tests;
