use axum::{
    routing::{get, post},
    Router,
};
use den_service::DenState;

mod auth;
mod events;
mod methods;
mod obligation_expiry;
mod rpc;

pub use obligation_expiry::{expire_client_obligations_once, run_client_obligation_expiry_loop};

pub fn router() -> Router<DenState> {
    Router::new()
        .route("/v1/rpc", post(rpc::rpc))
        .route(
            "/v1/sessions/{session_id}/events/page",
            get(events::events_page),
        )
}
