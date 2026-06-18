//! OAuth 2.0 authorization server + shared bearer-auth layer for the den HTTP edges.
//!
//! Extracted from `den-api` (v1.5+) so it sits *below* all the HTTP edges
//! (`den-api`, `den-web`, `den-acp`). Because `den-api` mounts the ACP surface and
//! both need OAuth (bearer-token auth + scopes), keeping OAuth here breaks the
//! `den-acp ↔ den-api` cycle that an `DenState`-in-`den-acp` split would otherwise
//! create. Contents:
//! - [`oauth`] — the OAuth 2.0 authorization server (endpoints, db, jwt, scopes).
//! - [`auth`] — `ApiError`, `BearerPrincipal`, and the bearer token/scope helpers.
//! - [`templates`] — the OAuth consent/authorize + error page rendering (its own
//!   `minijinja` template group, embedded in production via `build.rs`).
pub mod auth;
pub mod oauth;
pub mod templates;
