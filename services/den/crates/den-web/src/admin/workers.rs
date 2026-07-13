use axum::{extract::State, response::Response, routing::get, Router};
use axum_extra::routing::RouterExt;

use crate::errors::CustomError;
use crate::web::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route_with_tsr("/", get(index))
}

pub async fn index(state: State<AppState>) -> Result<Response, CustomError> {
    super::reflections::index(state).await
}
