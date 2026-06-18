use axum::{routing::{get, post}, Router};
use den_runtime::DenState;

mod auth;
mod events;
mod methods;
mod rpc;

pub fn router() -> Router<DenState> {
    Router::new()
        .route("/v1/rpc", post(rpc::rpc))
        .route("/v1/sessions/{session_id}/events", get(events::events))
}
