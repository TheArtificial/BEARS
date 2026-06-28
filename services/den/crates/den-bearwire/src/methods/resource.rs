use axum::http::HeaderMap;
use serde_json::{json, Value};

use den_http::errors::CustomError;
use den_service::DenState;
use den_runtime::{
    bearwire_events,
    runtime::bearwire_projection::wire::BearWireEvent,
};

use crate::auth::authenticated_bear;
use crate::methods::required_param_string;

pub(crate) async fn resource_update_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let resource = params
        .get("resource")
        .cloned()
        .or_else(|| params.get("payload").cloned())
        .unwrap_or_else(|| json!({}));
    let resource_kind = resource
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let mut event = BearWireEvent::ephemeral(
        "resource.updated",
        json!({
            "session_id": session_id,
            "resource": resource,
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    event.subject = Some(format!("resource/{resource_kind}"));
    let persisted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;

    Ok(json!({
        "ok": true,
        "event_sequence": persisted.sequence_no,
    }))
}
