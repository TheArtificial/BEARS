use axum::{extract::State, response::Response, routing::get, Router};

use crate::errors::CustomError;
use crate::web::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(index))
}

pub async fn index(state: State<AppState>) -> Result<Response, CustomError> {
    super::reflections::index(state).await
}
