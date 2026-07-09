//! Sandbox provider service (`RUN_SANDBOX`) and its Den-side HTTP client.
//!
//! The provider hosts isolated workspaces ("sandboxes") in which headless
//! bear-armatures execute Docket work. It is designed to run standalone on a
//! host separate from the Den instance using it: no database, no shared state
//! beyond its own filesystem and container runtime. Den talks to it over the
//! `/sandbox/v1` HTTP API; in-sandbox armatures dial back to Den over BearWire.

pub mod backend;
pub mod client;
pub mod metrics;
pub mod policy;
pub mod proc;
pub mod protocol;
pub mod recognize;
pub mod roots;
pub mod server;

pub use client::{SandboxClient, SandboxClientError};
pub use server::{create_sandbox_app, SandboxServerConfig};
