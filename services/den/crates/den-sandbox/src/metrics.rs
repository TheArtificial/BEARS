//! Crate-local counters exposed as Prometheus text at `/sandbox/v1/metrics`.
//! Deliberately self-contained (plain atomics, no registry): the provider must
//! run standalone, and den-core's metrics module is chat-runtime-specific.

use std::sync::atomic::{AtomicU64, Ordering};

static PROVISIONED_TOTAL: AtomicU64 = AtomicU64::new(0);
static FAILED_TOTAL: AtomicU64 = AtomicU64::new(0);
static DESTROYED_TOTAL: AtomicU64 = AtomicU64::new(0);
static TIMED_OUT_TOTAL: AtomicU64 = AtomicU64::new(0);
static CLEANUP_FAILURES_TOTAL: AtomicU64 = AtomicU64::new(0);
static LOG_BYTES_SERVED_TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn sandbox_provisioned() {
    PROVISIONED_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn sandbox_failed() {
    FAILED_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn sandbox_destroyed() {
    DESTROYED_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn sandbox_timed_out() {
    TIMED_OUT_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn cleanup_failure() {
    CLEANUP_FAILURES_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn log_bytes_served(bytes: u64) {
    LOG_BYTES_SERVED_TOTAL.fetch_add(bytes, Ordering::Relaxed);
}

pub fn render_prometheus_text(active_sandboxes: usize) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let counters = [
        (
            "den_sandbox_provisioned_total",
            PROVISIONED_TOTAL.load(Ordering::Relaxed),
        ),
        ("den_sandbox_failed_total", FAILED_TOTAL.load(Ordering::Relaxed)),
        (
            "den_sandbox_destroyed_total",
            DESTROYED_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "den_sandbox_timed_out_total",
            TIMED_OUT_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "den_sandbox_cleanup_failures_total",
            CLEANUP_FAILURES_TOTAL.load(Ordering::Relaxed),
        ),
        (
            "den_sandbox_log_bytes_served_total",
            LOG_BYTES_SERVED_TOTAL.load(Ordering::Relaxed),
        ),
    ];
    for (name, value) in counters {
        writeln!(s, "# TYPE {name} counter").unwrap();
        writeln!(s, "{name} {value}").unwrap();
    }
    writeln!(s, "# TYPE den_sandbox_active gauge").unwrap();
    writeln!(s, "den_sandbox_active {active_sandboxes}").unwrap();
    s
}
