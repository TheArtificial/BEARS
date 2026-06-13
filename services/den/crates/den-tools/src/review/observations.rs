//! `observation_write` tool — orchestration layer (watch-only).
//!
//! Runtime-agnostic: gating, argument validation, idempotency branch, and
//! payload shaping over the [`MemoryReviewStore`] seam. The `den` impl owns the
//! proposal creation + curate enqueue.

use den_core::{BearProfile, DenError};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    context::DenToolInvocationContext,
    support::{clean_optional, validate_bounded_text, validate_optional_object},
};

use super::store::{MemoryReviewStore, ObservationRecord, ObservationWriteRequest};

#[derive(Debug, Deserialize)]
pub struct ObservationWriteArguments {
    #[serde(default)]
    pub observation_id: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub salience: Option<String>,
    #[serde(default)]
    pub payload_ref: Option<String>,
    #[serde(default)]
    pub source: Option<Value>,
}

fn normalize_observation_id(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("obs-{}", Uuid::new_v4().as_simple()))
}

fn normalize_salience(raw: Option<&str>) -> Result<&'static str, DenError> {
    let salience = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("normal")
        .to_ascii_lowercase();
    match salience.as_str() {
        "low" => Ok("low"),
        "normal" => Ok("normal"),
        "high" => Ok("high"),
        "critical" => Ok("critical"),
        _ => Err(DenError::ValidationError(
            "salience must be low, normal, high, or critical".to_string(),
        )),
    }
}

fn observation_write_payload(observation: &ObservationRecord, idempotent: bool) -> Value {
    json!({
        "bear_id": observation.bear_id,
        "observation_id": observation.observation_id,
        "summary": observation.summary,
        "salience": observation.salience,
        "payload_ref": observation.payload_ref,
        "logical_path": observation.logical_path,
        "status": observation.status,
        "proposal_id": observation.proposal_id,
        "idempotent_replay": idempotent,
        "note": "Observation stored in Den and queued for memory_curate review.",
    })
}

pub async fn write_observation(
    store: &impl MemoryReviewStore,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    if role != BearProfile::Watch {
        return Err(DenError::Authorization(
            "den.observation.write is available only to watch".to_string(),
        ));
    }
    let args: ObservationWriteArguments = serde_json::from_value(arguments)?;
    let observation_id = normalize_observation_id(args.observation_id.as_deref());
    let summary = validate_bounded_text("summary", &args.summary, 1, 8_000)?;
    let salience = normalize_salience(args.salience.as_deref())?;
    let payload_ref = args
        .payload_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    validate_optional_object("source", &args.source)?;

    if let Some(existing) = store.find_observation(context.bear_id, &observation_id).await? {
        return Ok(observation_write_payload(&existing, true));
    }

    let source = args.source.unwrap_or_else(|| {
        json!({
            "binding_id": context.binding_id,
            "conversation_id": clean_optional(&context.conversation_id),
            "session_id": clean_optional(&context.session_id),
            "request_id": context.request_id,
        })
    });

    let record = store
        .record_observation(ObservationWriteRequest {
            bear_id: context.bear_id,
            binding_id: context.binding_id.clone(),
            observation_id,
            summary,
            salience: salience.to_string(),
            payload_ref,
            source,
            conversation_id: clean_optional(&context.conversation_id),
            session_id: clean_optional(&context.session_id),
            request_id: context.request_id.clone(),
        })
        .await?;

    Ok(observation_write_payload(&record, false))
}
