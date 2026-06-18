//! Compatibility shim — bear web-fetch approval/source policy now lives in the
//! `den-http` foundation crate (v1.5 split) so the api edge and the web handlers
//! can share it. Re-exported here so existing `crate::core::tools::web_policy::…`
//! and `crate::core::web_policy::…` paths keep resolving unchanged.
pub use den_http::web_policy::*;
