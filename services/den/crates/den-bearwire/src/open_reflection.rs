use std::time::Duration;

use den_http::errors::CustomError;
use den_service::DenState;
use tokio_util::sync::CancellationToken;

pub async fn run_open_session_reflection_loop(
    state: DenState,
    token: CancellationToken,
    interval: Duration,
) -> Result<(), CustomError> {
    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = token.cancelled() => return Ok(()),
            _ = ticker.tick() => {
                let processed = crate::methods::session::reflect_open_sessions_once(&state).await?;
                if processed > 0 {
                    tracing::info!(processed, "Workers: open-session pair reflection sweep processed sessions");
                }
            }
        }
    }
}
