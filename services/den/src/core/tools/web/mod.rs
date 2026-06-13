//! Web tools — `den` boundary.
//!
//! The orchestration lives in `den_tools::web`; this module only provides the
//! concrete [`WebFetcher`](den_tools::web::WebFetcher) implementation
//! (`runtime`) and thin wrappers that build it and map `DenError` to the
//! web-boundary `CustomError`. See docs/roadmap/DEN_CRATE_SPLIT_PLAN.md (Phase B).

mod runtime;

use serde_json::Value;
use sqlx::PgPool;

use crate::{config::Config, core::tools::session::DenToolInvocationContext, errors::CustomError};
use runtime::DenWebFetcher;

pub(crate) async fn web_fetch(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let fetcher = DenWebFetcher { pool, config };
    den_tools::web::web_fetch(&fetcher, context.bear_id, &context.session_id, arguments)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn web_search(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, CustomError> {
    let fetcher = DenWebFetcher { pool, config };
    den_tools::web::web_search(&fetcher, Some(context.bear_id), arguments)
        .await
        .map_err(CustomError::from)
}
