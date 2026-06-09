use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        bear_observations::{self, BearObservationRow},
        bears::BearProfile,
        memory::{
            create_observation, create_proposal, get_observation,
            mark_observation_review_queued_for_bear, MemoryStoreManager,
        },
        memory_proposals::{self, CreateMemoryProposal},
        reflection_conductor::{self, ProposalEnqueueParams},
        tools::{
            session::DenToolInvocationContext,
            support::{clean_optional, validate_bounded_text, validate_optional_object},
        },
    },
    errors::CustomError,
};

#[derive(Debug, Deserialize)]
pub(crate) struct ObservationWriteArguments {
    #[serde(default)]
    pub(crate) observation_id: Option<String>,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) salience: Option<String>,
    #[serde(default)]
    pub(crate) payload_ref: Option<String>,
    #[serde(default)]
    pub(crate) source: Option<Value>,
}

fn normalize_observation_id(raw: Option<String>) -> String {
    raw.as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("obs-{}", Uuid::new_v4().as_simple()))
}

fn normalize_salience(raw: Option<String>) -> Result<&'static str, CustomError> {
    let salience = raw
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("normal")
        .to_ascii_lowercase();
    match salience.as_str() {
        "low" | "normal" | "high" | "critical" => Ok(match salience.as_str() {
            "low" => "low",
            "normal" => "normal",
            "high" => "high",
            _ => "critical",
        }),
        _ => Err(CustomError::ValidationError(
            "salience must be low, normal, high, or critical".to_string(),
        )),
    }
}

fn observation_requires_human(salience: &str) -> bool {
    matches!(salience, "high" | "critical")
}

pub(crate) async fn write_observation(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearProfile::Watch {
        return Err(CustomError::Authorization(
            "den.observation.write is available only to watch".to_string(),
        ));
    }
    let args: ObservationWriteArguments = serde_json::from_value(arguments)?;
    let observation_id = normalize_observation_id(args.observation_id);
    let summary = validate_bounded_text("summary", &args.summary, 1, 8_000)?;
    let salience = normalize_salience(args.salience)?;
    let payload_ref = args
        .payload_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    validate_optional_object("source", &args.source)?;

    if let Some(existing) =
        get_observation(pool, config, stores, context.bear_id, &observation_id).await?
    {
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

    let observation = create_observation(
        pool,
        config,
        stores,
        bear_observations::CreateBearObservation {
            bear_id: context.bear_id,
            observation_id: &observation_id,
            summary: &summary,
            salience,
            payload_ref,
            source,
        },
    )
    .await?;

    let proposal = enqueue_observation_review(
        pool,
        config,
        stores,
        context,
        &observation,
        salience,
    )
    .await?;

    if config.uses_native_agent_runtime() {
        mark_observation_review_queued_for_bear(
            config,
            stores,
            context.bear_id,
            &observation.observation_id,
            proposal.id,
        )
        .await?;
        let mut observation = observation;
        observation.status = "review_queued".to_string();
        observation.proposal_id = Some(proposal.id);
        return Ok(observation_write_payload(&observation, false));
    }

    let observation = bear_observations::mark_review_queued(
        pool,
        context.bear_id,
        observation.id,
        proposal.id,
    )
    .await?;

    Ok(observation_write_payload(&observation, false))
}

async fn enqueue_observation_review(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    context: &DenToolInvocationContext,
    observation: &BearObservationRow,
    salience: &str,
) -> Result<memory_proposals::MemoryProposalRow, CustomError> {
    let requires_human = observation_requires_human(salience);
    let conversation_id = clean_optional(&context.conversation_id);
    let proposal = create_proposal(
        pool,
        config,
        stores,
        CreateMemoryProposal {
            bear_id: context.bear_id,
            source_role: BearProfile::Watch,
            source_agent_id: Some(context.binding_id.clone()),
            source_paths: vec![observation.logical_path.clone()],
            source_refs: serde_json::json!({
                "observation_id": observation.observation_id,
                "observation_row_id": observation.id,
                "conversation_id": conversation_id,
                "session_id": clean_optional(&context.session_id),
                "request_id": context.request_id,
            }),
            suggested_action: if requires_human {
                "human_review"
            } else {
                "unspecified"
            },
            target_ref: None,
            title: &format!("Review watch observation: {}", observation.observation_id),
            summary: observation.summary.as_str(),
            rationale: "Watch recorded an inbound observation that may warrant curate review.",
            proposed_content: None,
            proposed_patch: None,
            refs: serde_json::json!({
                "observation_id": observation.observation_id,
                "salience": salience,
                "payload_ref": observation.payload_ref,
                "logical_path": observation.logical_path,
            }),
            sensitivity: "normal",
            requires_human,
            project_to_conversation: conversation_id.is_some(),
        },
    )
    .await?;

    let reflection_date = OffsetDateTime::now_utc().date();
    let conversation_key = format!("memory_curate:{reflection_date}");
    reflection_conductor::enqueue_memory_curate_for_proposals(
        pool,
        ProposalEnqueueParams {
            bear_id: context.bear_id,
            binding_id: Some(context.binding_id.as_str()),
            conversation_id: conversation_id.as_deref(),
            conversation_key: Some(&conversation_key),
            conversation_date: Some(reflection_date),
            trigger: "watch_observation",
            proposal_ids: vec![proposal.id],
        },
    )
    .await?;

    Ok(proposal)
}

fn observation_write_payload(observation: &BearObservationRow, idempotent: bool) -> Value {
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
