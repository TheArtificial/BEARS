//! Lightweight observability: hand-rolled Prometheus text metrics and chat SSE proxy instrumentation.
pub mod chat_proxy_stream;
pub use den_core::metrics;
pub mod native_web_chat_stream;
