use den_core::DenError;
use den_protocol::ContextBudgetReport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{RuntimeCheckpointRequest, RuntimeCheckpointResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointValidationStatus {
    Requested,
    Valid,
    Invalid,
    Superseded,
}

impl CheckpointValidationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointVisibility {
    AuditOnly,
    LiveEphemeral,
    ModelVisibleHidden,
}

impl CheckpointVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuditOnly => "audit_only",
            Self::LiveEphemeral => "live_ephemeral",
            Self::ModelVisibleHidden => "model_visible_hidden",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReplayPolicy {
    None,
    SummaryOnce,
    UntilSuperseded,
}

impl CheckpointReplayPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SummaryOnce => "summary_once",
            Self::UntilSuperseded => "until_superseded",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CheckpointArtifactInput {
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub orientation_kind: Option<String>,
    pub request: RuntimeCheckpointRequest,
    pub visibility: CheckpointVisibility,
    pub replay_policy: CheckpointReplayPolicy,
}

#[derive(Debug, Clone)]
pub struct CheckpointResponseInput {
    pub run_id: String,
    pub checkpoint_id: String,
    pub response: RuntimeCheckpointResponse,
    pub validation_status: CheckpointValidationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointArtifactRow {
    pub id: Uuid,
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub checkpoint_id: String,
    pub reason: String,
    pub control_level: String,
    pub request: Value,
    pub response: Option<Value>,
    pub validation_status: String,
    pub visibility: String,
    pub replay_policy: String,
    pub related_task_list_id: Option<String>,
    pub related_task_item_id: Option<String>,
    pub related_docket_task_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopControlDecisionKind {
    CheckpointRequested,
    ContextBudgetPressure,
    GroundingProbeResult,
}

impl LoopControlDecisionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CheckpointRequested => "checkpoint_requested",
            Self::ContextBudgetPressure => "context_budget_pressure",
            Self::GroundingProbeResult => "grounding_probe_result",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "checkpoint_requested" => Some(Self::CheckpointRequested),
            "context_budget_pressure" => Some(Self::ContextBudgetPressure),
            "grounding_probe_result" => Some(Self::GroundingProbeResult),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextBudgetPressureLevel {
    NearBudget,
    OverBudget,
}

impl ContextBudgetPressureLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NearBudget => "near_budget",
            Self::OverBudget => "over_budget",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContextBudgetPressureDecision {
    level: ContextBudgetPressureLevel,
    model: String,
    context_window: Option<u32>,
    estimated_input_tokens: u32,
    reserved_output_tokens: u32,
    estimated_total_tokens: u32,
    near_budget: bool,
    over_budget: bool,
    action: &'static str,
}

impl ContextBudgetPressureDecision {
    fn from_report(report: &ContextBudgetReport) -> Option<Self> {
        let level = if report.over_budget {
            ContextBudgetPressureLevel::OverBudget
        } else if report.near_budget {
            ContextBudgetPressureLevel::NearBudget
        } else {
            return None;
        };
        Some(Self {
            level,
            model: report.model.clone(),
            context_window: report.context_window,
            estimated_input_tokens: report.estimated_input_tokens,
            reserved_output_tokens: report.reserved_output_tokens,
            estimated_total_tokens: report.estimated_total_tokens,
            near_budget: report.near_budget,
            over_budget: report.over_budget,
            action: context_budget_pressure_action(level),
        })
    }
}

pub const fn context_budget_pressure_action(level: ContextBudgetPressureLevel) -> &'static str {
    match level {
        ContextBudgetPressureLevel::NearBudget => "prefer_checkpoint_before_more_context_growth",
        ContextBudgetPressureLevel::OverBudget => {
            "stop_before_model_call_and_request_compaction_or_smaller_context"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingProbeSignalKind {
    Pass,
    Fail,
    NoSignal,
}

impl GroundingProbeSignalKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::NoSignal => "no_signal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pass" => Some(Self::Pass),
            "fail" => Some(Self::Fail),
            "no_signal" => Some(Self::NoSignal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundingProbeFinding {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingProbeResultInput {
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub orientation_kind: Option<String>,
    pub tool_call_id: Option<String>,
    pub probe_id: String,
    pub surface_kind: String,
    pub signal: GroundingProbeSignalKind,
    pub duration_ms: u64,
    pub findings: Vec<GroundingProbeFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GroundingProbeDecision {
    probe_id: String,
    surface_kind: String,
    signal: GroundingProbeSignalKind,
    duration_ms: u64,
    findings: Vec<GroundingProbeFinding>,
}

pub fn non_empty_diff_grounding_probe(
    run_id: impl Into<String>,
    diff: &str,
) -> GroundingProbeResultInput {
    let changed = !diff.trim().is_empty();
    GroundingProbeResultInput {
        run_id: run_id.into(),
        turn_step_id: None,
        orientation_kind: None,
        tool_call_id: None,
        probe_id: "generic.non_empty_diff".to_string(),
        surface_kind: "repository".to_string(),
        signal: if changed {
            GroundingProbeSignalKind::Pass
        } else {
            GroundingProbeSignalKind::Fail
        },
        duration_ms: 0,
        findings: vec![GroundingProbeFinding {
            code: if changed {
                "diff_present"
            } else {
                "empty_diff"
            }
            .to_string(),
            message: if changed {
                "Workspace diff is non-empty."
            } else {
                "Workspace diff is empty."
            }
            .to_string(),
        }],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopControlLedgerInput {
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub decision_id: String,
    pub decision_kind: LoopControlDecisionKind,
    pub control_level: String,
    pub reason: Option<String>,
    pub orientation_kind: Option<String>,
    pub checkpoint_id: Option<String>,
    pub related_task_list_id: Option<String>,
    pub related_task_item_id: Option<String>,
    pub related_docket_job_id: Option<Uuid>,
    pub related_docket_task_id: Option<Uuid>,
    pub evidence_refs: Vec<LedgerEvidenceRef>,
    pub decision: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEvidenceRef {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopControlLedgerRow {
    pub id: Uuid,
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub decision_id: String,
    pub decision_kind: String,
    pub control_level: String,
    pub reason: Option<String>,
    pub orientation_kind: Option<String>,
    pub checkpoint_id: Option<String>,
    pub related_task_list_id: Option<String>,
    pub related_task_item_id: Option<String>,
    pub related_docket_job_id: Option<Uuid>,
    pub related_docket_task_id: Option<Uuid>,
    pub evidence_refs: Value,
    pub decision: Value,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopControlReplayObservation {
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub decision_id: String,
    pub decision_kind: LoopControlDecisionKind,
    pub control_level: String,
    pub reason: Option<String>,
    pub orientation_kind: Option<String>,
    pub checkpoint_id: Option<String>,
    pub related_task_list_id: Option<String>,
    pub related_task_item_id: Option<String>,
    pub related_docket_job_id: Option<Uuid>,
    pub related_docket_task_id: Option<Uuid>,
    pub evidence_refs: Vec<LedgerEvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopControlReplayMismatch {
    pub index: usize,
    pub expected: Option<LoopControlReplayObservation>,
    pub observed: Option<LoopControlReplayObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopControlReplayTurn {
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub decision_count: usize,
    pub decision_ids: Vec<String>,
    pub decision_kinds: Vec<LoopControlDecisionKind>,
    pub control_levels: Vec<String>,
    pub reasons: Vec<String>,
    pub orientation_kinds: Vec<String>,
    pub checkpoint_ids: Vec<String>,
    pub related_task_list_ids: Vec<String>,
    pub related_task_item_ids: Vec<String>,
    pub related_docket_job_ids: Vec<Uuid>,
    pub related_docket_task_ids: Vec<Uuid>,
    pub evidence_refs: Vec<LedgerEvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedLoopControlReplayTurn {
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub decision_count: usize,
    pub decision_kinds: Vec<LoopControlDecisionKind>,
    pub control_levels: Vec<String>,
    pub reasons: Vec<String>,
    pub orientation_kinds: Vec<String>,
    pub checkpoint_ids: Vec<String>,
    pub evidence_refs: Vec<LedgerEvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopControlReplayTurnMismatch {
    pub index: usize,
    pub expected: Option<ExpectedLoopControlReplayTurn>,
    pub observed: Option<LoopControlReplayTurn>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopControlReplayProfileSummary {
    pub turn_count: usize,
    pub decision_count: usize,
    pub decision_kind_counts: Vec<LoopControlReplayCount<LoopControlDecisionKind>>,
    pub control_level_counts: Vec<LoopControlReplayCount<String>>,
    pub orientation_kind_counts: Vec<LoopControlReplayCount<String>>,
    pub reason_counts: Vec<LoopControlReplayCount<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedLoopControlReplayProfileSummary {
    pub turn_count: usize,
    pub decision_count: usize,
    pub decision_kind_counts: Vec<LoopControlReplayCount<LoopControlDecisionKind>>,
    pub control_level_counts: Vec<LoopControlReplayCount<String>>,
    pub orientation_kind_counts: Vec<LoopControlReplayCount<String>>,
    pub reason_counts: Vec<LoopControlReplayCount<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopControlReplayProfileMismatch {
    pub expected: ExpectedLoopControlReplayProfileSummary,
    pub observed: LoopControlReplayProfileSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopControlReplayCount<T> {
    pub value: T,
    pub count: usize,
}

pub fn replay_loop_control_observations(
    rows: &[LoopControlLedgerRow],
) -> Result<Vec<LoopControlReplayObservation>, DenError> {
    rows.iter().map(replay_observation_from_row).collect()
}

pub fn compare_loop_control_replay(
    observed: &[LoopControlReplayObservation],
    expected: &[LoopControlReplayObservation],
) -> Vec<LoopControlReplayMismatch> {
    let max_len = observed.len().max(expected.len());
    (0..max_len)
        .filter_map(|index| {
            let observed = observed.get(index).cloned();
            let expected = expected.get(index).cloned();
            (observed != expected).then_some(LoopControlReplayMismatch {
                index,
                expected,
                observed,
            })
        })
        .collect()
}

pub fn aggregate_loop_control_replay_turns(
    rows: &[LoopControlLedgerRow],
) -> Result<Vec<LoopControlReplayTurn>, DenError> {
    let observations = replay_loop_control_observations(rows)?;
    let mut turns: Vec<LoopControlReplayTurn> = Vec::new();
    for observation in observations {
        // ponytail: linear grouping is fine for per-run replay fixtures; upgrade to
        // an index map if replaying large production windows in-process.
        let turn = match turns.iter_mut().find(|turn| {
            turn.run_id == observation.run_id && turn.turn_step_id == observation.turn_step_id
        }) {
            Some(turn) => turn,
            None => {
                turns.push(LoopControlReplayTurn {
                    run_id: observation.run_id.clone(),
                    turn_step_id: observation.turn_step_id,
                    decision_count: 0,
                    decision_ids: Vec::new(),
                    decision_kinds: Vec::new(),
                    control_levels: Vec::new(),
                    reasons: Vec::new(),
                    orientation_kinds: Vec::new(),
                    checkpoint_ids: Vec::new(),
                    related_task_list_ids: Vec::new(),
                    related_task_item_ids: Vec::new(),
                    related_docket_job_ids: Vec::new(),
                    related_docket_task_ids: Vec::new(),
                    evidence_refs: Vec::new(),
                });
                turns.last_mut().expect("turn was just pushed")
            }
        };
        turn.decision_count += 1;
        push_unique(&mut turn.decision_ids, observation.decision_id);
        push_unique(&mut turn.decision_kinds, observation.decision_kind);
        push_unique(&mut turn.control_levels, observation.control_level);
        push_optional_unique(&mut turn.reasons, observation.reason);
        push_optional_unique(&mut turn.orientation_kinds, observation.orientation_kind);
        push_optional_unique(&mut turn.checkpoint_ids, observation.checkpoint_id);
        push_optional_unique(
            &mut turn.related_task_list_ids,
            observation.related_task_list_id,
        );
        push_optional_unique(
            &mut turn.related_task_item_ids,
            observation.related_task_item_id,
        );
        push_optional_unique(
            &mut turn.related_docket_job_ids,
            observation.related_docket_job_id,
        );
        push_optional_unique(
            &mut turn.related_docket_task_ids,
            observation.related_docket_task_id,
        );
        for evidence_ref in observation.evidence_refs {
            push_unique(&mut turn.evidence_refs, evidence_ref);
        }
    }
    Ok(turns)
}

pub fn compare_loop_control_replay_turns(
    observed: &[LoopControlReplayTurn],
    expected: &[ExpectedLoopControlReplayTurn],
) -> Vec<LoopControlReplayTurnMismatch> {
    let max_len = observed.len().max(expected.len());
    (0..max_len)
        .filter_map(|index| {
            let observed = observed.get(index).cloned();
            let expected = expected.get(index).cloned();
            let matches = match (&observed, &expected) {
                (Some(observed), Some(expected)) => {
                    replay_turn_matches_expected(observed, expected)
                }
                (None, None) => true,
                _ => false,
            };
            (!matches).then_some(LoopControlReplayTurnMismatch {
                index,
                expected,
                observed,
            })
        })
        .collect()
}

fn replay_turn_matches_expected(
    observed: &LoopControlReplayTurn,
    expected: &ExpectedLoopControlReplayTurn,
) -> bool {
    observed.run_id == expected.run_id
        && observed.turn_step_id == expected.turn_step_id
        && observed.decision_count == expected.decision_count
        && observed.decision_kinds == expected.decision_kinds
        && observed.control_levels == expected.control_levels
        && observed.reasons == expected.reasons
        && observed.orientation_kinds == expected.orientation_kinds
        && observed.checkpoint_ids == expected.checkpoint_ids
        && observed.evidence_refs == expected.evidence_refs
}

pub fn summarize_loop_control_replay_profile(
    turns: &[LoopControlReplayTurn],
) -> LoopControlReplayProfileSummary {
    let mut decision_kind_counts = Vec::new();
    let mut control_level_counts = Vec::new();
    let mut orientation_kind_counts = Vec::new();
    let mut reason_counts = Vec::new();
    for turn in turns {
        for decision_kind in &turn.decision_kinds {
            increment_replay_count(&mut decision_kind_counts, *decision_kind);
        }
        for control_level in &turn.control_levels {
            increment_replay_count(&mut control_level_counts, control_level.clone());
        }
        for orientation_kind in &turn.orientation_kinds {
            increment_replay_count(&mut orientation_kind_counts, orientation_kind.clone());
        }
        for reason in &turn.reasons {
            increment_replay_count(&mut reason_counts, reason.clone());
        }
    }
    LoopControlReplayProfileSummary {
        turn_count: turns.len(),
        decision_count: turns.iter().map(|turn| turn.decision_count).sum(),
        decision_kind_counts,
        control_level_counts,
        orientation_kind_counts,
        reason_counts,
    }
}

pub fn compare_loop_control_replay_profile(
    observed: &LoopControlReplayProfileSummary,
    expected: &ExpectedLoopControlReplayProfileSummary,
) -> Option<LoopControlReplayProfileMismatch> {
    let matches = observed.turn_count == expected.turn_count
        && observed.decision_count == expected.decision_count
        && observed.decision_kind_counts == expected.decision_kind_counts
        && observed.control_level_counts == expected.control_level_counts
        && observed.orientation_kind_counts == expected.orientation_kind_counts
        && observed.reason_counts == expected.reason_counts;
    (!matches).then_some(LoopControlReplayProfileMismatch {
        expected: expected.clone(),
        observed: observed.clone(),
    })
}

fn increment_replay_count<T: PartialEq>(counts: &mut Vec<LoopControlReplayCount<T>>, value: T) {
    // ponytail: vector counters preserve first-seen order and are enough for small replay
    // windows; use BTreeMap if offline replay starts scanning large production slices.
    match counts.iter_mut().find(|count| count.value == value) {
        Some(count) => count.count += 1,
        None => counts.push(LoopControlReplayCount { value, count: 1 }),
    }
}

fn push_optional_unique<T: PartialEq>(values: &mut Vec<T>, value: Option<T>) {
    if let Some(value) = value {
        push_unique(values, value);
    }
}

fn push_unique<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn replay_observation_from_row(
    row: &LoopControlLedgerRow,
) -> Result<LoopControlReplayObservation, DenError> {
    let decision_kind = LoopControlDecisionKind::parse(&row.decision_kind).ok_or_else(|| {
        DenError::System(format!(
            "unknown loop-control decision kind in replay ledger: {}",
            row.decision_kind
        ))
    })?;
    let evidence_refs: Vec<LedgerEvidenceRef> =
        serde_json::from_value(row.evidence_refs.clone())
            .map_err(|err| DenError::System(format!("deserialize replay evidence refs: {err}")))?;
    Ok(LoopControlReplayObservation {
        run_id: row.run_id.clone(),
        turn_step_id: row.turn_step_id,
        decision_id: row.decision_id.clone(),
        decision_kind,
        control_level: row.control_level.clone(),
        reason: row.reason.clone(),
        orientation_kind: row.orientation_kind.clone(),
        checkpoint_id: row.checkpoint_id.clone(),
        related_task_list_id: row.related_task_list_id.clone(),
        related_task_item_id: row.related_task_item_id.clone(),
        related_docket_job_id: row.related_docket_job_id,
        related_docket_task_id: row.related_docket_task_id,
        evidence_refs,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointLedgerDecision {
    checkpoint_id: String,
    reason: String,
    control_level: String,
    profile_fingerprint: Option<String>,
    active_objective_present: bool,
    required_fields: Vec<String>,
    task_refs: CheckpointLedgerTaskRefs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointLedgerTaskRefs {
    task_list_id: Option<String>,
    task_list_version: Option<String>,
    active_item_id: Option<String>,
    docket_job_id: Option<String>,
    docket_task_id: Option<String>,
}

pub async fn record_checkpoint_request(
    pool: &PgPool,
    input: CheckpointArtifactInput,
) -> Result<CheckpointArtifactRow, DenError> {
    let request_json = serde_json::to_value(&input.request)
        .map_err(|err| DenError::System(format!("serialize checkpoint request: {err}")))?;
    let task_context = input.request.task_context.as_ref();
    let related_docket_task_id = task_context
        .and_then(|context| context.docket_task_id.as_deref())
        .and_then(|value| Uuid::parse_str(value).ok());

    let row = sqlx::query(
        r"
        INSERT INTO bear_run_checkpoints (
            run_id, turn_step_id, checkpoint_id, reason, control_level, request,
            validation_status, visibility, replay_policy, related_task_list_id,
            related_task_item_id, related_docket_task_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, 'requested', $7, $8, $9, $10, $11)
        ON CONFLICT (run_id, checkpoint_id) DO UPDATE SET
            request = EXCLUDED.request,
            reason = EXCLUDED.reason,
            control_level = EXCLUDED.control_level,
            validation_status = 'requested',
            visibility = EXCLUDED.visibility,
            replay_policy = EXCLUDED.replay_policy,
            related_task_list_id = EXCLUDED.related_task_list_id,
            related_task_item_id = EXCLUDED.related_task_item_id,
            related_docket_task_id = EXCLUDED.related_docket_task_id,
            response = NULL,
            updated_at = NOW()
        RETURNING
            id, run_id, turn_step_id, checkpoint_id, reason, control_level, request,
            response, validation_status, visibility, replay_policy, related_task_list_id,
            related_task_item_id, related_docket_task_id, created_at, updated_at
        ",
    )
    .bind(&input.run_id)
    .bind(input.turn_step_id)
    .bind(&input.request.checkpoint_id)
    .bind(input.request.reason.as_str())
    .bind(input.request.control_level.as_str())
    .bind(request_json)
    .bind(input.visibility.as_str())
    .bind(input.replay_policy.as_str())
    .bind(task_context.and_then(|context| context.task_list_id.as_deref()))
    .bind(task_context.and_then(|context| context.active_item_id.as_deref()))
    .bind(related_docket_task_id)
    .fetch_one(pool)
    .await?;

    let checkpoint = row_to_checkpoint(row);
    record_loop_control_decision(
        pool,
        checkpoint_request_ledger_input(
            &input.request,
            input.turn_step_id,
            input.orientation_kind.clone(),
        )?,
    )
    .await?;
    Ok(checkpoint)
}

pub async fn record_context_budget_pressure_decision(
    pool: &PgPool,
    run_id: &str,
    turn_step_id: Option<Uuid>,
    orientation_kind: Option<String>,
    report: &ContextBudgetReport,
) -> Result<Option<LoopControlLedgerRow>, DenError> {
    let Some(decision) = ContextBudgetPressureDecision::from_report(report) else {
        return Ok(None);
    };
    let level = decision.level;
    let decision_json = serde_json::to_value(&decision)
        .map_err(|err| DenError::System(format!("serialize context budget decision: {err}")))?;
    let row = record_loop_control_decision(
        pool,
        LoopControlLedgerInput {
            run_id: run_id.to_string(),
            turn_step_id,
            decision_id: format!("context_budget:{}:{turn_step_id:?}", level.as_str()),
            decision_kind: LoopControlDecisionKind::ContextBudgetPressure,
            control_level: "standard".to_string(),
            reason: Some(level.as_str().to_string()),
            orientation_kind,
            checkpoint_id: None,
            related_task_list_id: None,
            related_task_item_id: None,
            related_docket_job_id: None,
            related_docket_task_id: None,
            evidence_refs: vec![LedgerEvidenceRef {
                kind: "context_budget_report".to_string(),
                id: format!(
                    "{}:{}:{}",
                    report.model,
                    report.estimated_total_tokens,
                    report.context_window.unwrap_or_default()
                ),
            }],
            decision: decision_json,
        },
    )
    .await?;
    Ok(Some(row))
}

pub async fn record_grounding_probe_result_decision(
    pool: &PgPool,
    input: GroundingProbeResultInput,
) -> Result<LoopControlLedgerRow, DenError> {
    let decision = GroundingProbeDecision {
        probe_id: input.probe_id.clone(),
        surface_kind: input.surface_kind.clone(),
        signal: input.signal,
        duration_ms: input.duration_ms,
        findings: input.findings,
    };
    let decision_json = serde_json::to_value(&decision)
        .map_err(|err| DenError::System(format!("serialize grounding probe decision: {err}")))?;
    let mut evidence_refs = vec![LedgerEvidenceRef {
        kind: "grounding_probe".to_string(),
        id: input.probe_id.clone(),
    }];
    if let Some(tool_call_id) = input.tool_call_id.as_ref() {
        evidence_refs.push(LedgerEvidenceRef {
            kind: "tool_call".to_string(),
            id: tool_call_id.clone(),
        });
    }
    record_loop_control_decision(
        pool,
        LoopControlLedgerInput {
            run_id: input.run_id,
            turn_step_id: input.turn_step_id,
            decision_id: format!("grounding_probe:{}", input.probe_id),
            decision_kind: LoopControlDecisionKind::GroundingProbeResult,
            control_level: "standard".to_string(),
            reason: Some(input.signal.as_str().to_string()),
            orientation_kind: input.orientation_kind,
            checkpoint_id: None,
            related_task_list_id: None,
            related_task_item_id: None,
            related_docket_job_id: None,
            related_docket_task_id: None,
            evidence_refs,
            decision: decision_json,
        },
    )
    .await
}

pub async fn record_loop_control_decision(
    pool: &PgPool,
    input: LoopControlLedgerInput,
) -> Result<LoopControlLedgerRow, DenError> {
    let evidence_refs = serde_json::to_value(&input.evidence_refs)
        .map_err(|err| DenError::System(format!("serialize loop-control evidence refs: {err}")))?;
    let row = sqlx::query(
        r"
        INSERT INTO bear_loop_control_ledger (
            run_id, turn_step_id, decision_id, decision_kind, control_level, reason,
            orientation_kind, checkpoint_id, related_task_list_id, related_task_item_id,
            related_docket_job_id, related_docket_task_id, evidence_refs, decision
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        ON CONFLICT (run_id, decision_id) DO UPDATE SET
            turn_step_id = EXCLUDED.turn_step_id,
            decision_kind = EXCLUDED.decision_kind,
            control_level = EXCLUDED.control_level,
            reason = EXCLUDED.reason,
            orientation_kind = EXCLUDED.orientation_kind,
            checkpoint_id = EXCLUDED.checkpoint_id,
            related_task_list_id = EXCLUDED.related_task_list_id,
            related_task_item_id = EXCLUDED.related_task_item_id,
            related_docket_job_id = EXCLUDED.related_docket_job_id,
            related_docket_task_id = EXCLUDED.related_docket_task_id,
            evidence_refs = EXCLUDED.evidence_refs,
            decision = EXCLUDED.decision
        RETURNING
            id, run_id, turn_step_id, decision_id, decision_kind, control_level, reason,
            orientation_kind, checkpoint_id, related_task_list_id, related_task_item_id,
            related_docket_job_id, related_docket_task_id, evidence_refs, decision, created_at
        ",
    )
    .bind(&input.run_id)
    .bind(input.turn_step_id)
    .bind(&input.decision_id)
    .bind(input.decision_kind.as_str())
    .bind(&input.control_level)
    .bind(&input.reason)
    .bind(&input.orientation_kind)
    .bind(&input.checkpoint_id)
    .bind(&input.related_task_list_id)
    .bind(&input.related_task_item_id)
    .bind(input.related_docket_job_id)
    .bind(input.related_docket_task_id)
    .bind(evidence_refs)
    .bind(input.decision)
    .fetch_one(pool)
    .await?;

    Ok(row_to_ledger(row))
}

pub async fn list_loop_control_decisions_for_run(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<LoopControlLedgerRow>, DenError> {
    let rows = sqlx::query(
        r"
        SELECT
            id, run_id, turn_step_id, decision_id, decision_kind, control_level, reason,
            orientation_kind, checkpoint_id, related_task_list_id, related_task_item_id,
            related_docket_job_id, related_docket_task_id, evidence_refs, decision, created_at
        FROM bear_loop_control_ledger
        WHERE run_id = $1
        ORDER BY created_at ASC, decision_id ASC
        ",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_ledger).collect())
}

pub async fn latest_grounding_probe_signal_for_tool_call(
    pool: &PgPool,
    run_id: &str,
    tool_call_id: &str,
) -> Result<Option<GroundingProbeSignalKind>, DenError> {
    let evidence_ref = serde_json::json!([{ "kind": "tool_call", "id": tool_call_id }]);
    let row: Option<(Option<String>,)> = sqlx::query_as(
        r"
        SELECT reason
        FROM bear_loop_control_ledger
        WHERE run_id = $1
          AND decision_kind = 'grounding_probe_result'
          AND evidence_refs @> $2::jsonb
        ORDER BY created_at DESC, decision_id DESC
        LIMIT 1
        ",
    )
    .bind(run_id)
    .bind(evidence_ref)
    .fetch_optional(pool)
    .await?;

    Ok(row
        .and_then(|(reason,)| reason)
        .and_then(|reason| GroundingProbeSignalKind::parse(&reason)))
}

pub async fn latest_grounding_probe_signal_for_run(
    pool: &PgPool,
    run_id: &str,
) -> Result<Option<GroundingProbeSignalKind>, DenError> {
    // ponytail: this is a run-level signal because current probe rows are not yet
    // tied to individual tool-call ids; add a tool-call evidence ref if multiple
    // mutation probes can overlap within one continuation.
    let row: Option<(Option<String>,)> = sqlx::query_as(
        r"
        SELECT reason
        FROM bear_loop_control_ledger
        WHERE run_id = $1
          AND decision_kind = 'grounding_probe_result'
        ORDER BY created_at DESC, decision_id DESC
        LIMIT 1
        ",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(row
        .and_then(|(reason,)| reason)
        .and_then(|reason| GroundingProbeSignalKind::parse(&reason)))
}

pub async fn summarize_loop_control_replay_profile_for_run(
    pool: &PgPool,
    run_id: &str,
) -> Result<LoopControlReplayProfileSummary, DenError> {
    let rows = list_loop_control_decisions_for_run(pool, run_id).await?;
    let turns = aggregate_loop_control_replay_turns(&rows)?;
    Ok(summarize_loop_control_replay_profile(&turns))
}

fn checkpoint_request_ledger_input(
    request: &RuntimeCheckpointRequest,
    turn_step_id: Option<Uuid>,
    orientation_kind: Option<String>,
) -> Result<LoopControlLedgerInput, DenError> {
    let task_context = request.task_context.as_ref();
    let related_docket_job_id = task_context
        .and_then(|context| context.docket_job_id.as_deref())
        .and_then(|value| Uuid::parse_str(value).ok());
    let related_docket_task_id = task_context
        .and_then(|context| context.docket_task_id.as_deref())
        .and_then(|value| Uuid::parse_str(value).ok());
    let required_fields = request
        .required_fields
        .iter()
        .map(serde_json::to_value)
        .map(|value| {
            value
                .map_err(|err| DenError::System(format!("serialize checkpoint field: {err}")))
                .and_then(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        DenError::System("checkpoint field did not serialize to string".to_string())
                    })
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let decision = serde_json::to_value(CheckpointLedgerDecision {
        checkpoint_id: request.checkpoint_id.clone(),
        reason: request.reason.as_str().to_string(),
        control_level: request.control_level.as_str().to_string(),
        profile_fingerprint: request.profile_fingerprint.clone(),
        active_objective_present: request.active_objective.is_some(),
        required_fields,
        task_refs: CheckpointLedgerTaskRefs {
            task_list_id: task_context.and_then(|context| context.task_list_id.clone()),
            task_list_version: task_context.and_then(|context| context.task_list_version.clone()),
            active_item_id: task_context.and_then(|context| context.active_item_id.clone()),
            docket_job_id: task_context.and_then(|context| context.docket_job_id.clone()),
            docket_task_id: task_context.and_then(|context| context.docket_task_id.clone()),
        },
    })
    .map_err(|err| DenError::System(format!("serialize checkpoint ledger decision: {err}")))?;

    Ok(LoopControlLedgerInput {
        run_id: request.run_id.clone(),
        turn_step_id,
        decision_id: format!("checkpoint:{}", request.checkpoint_id),
        decision_kind: LoopControlDecisionKind::CheckpointRequested,
        control_level: request.control_level.as_str().to_string(),
        reason: Some(request.reason.as_str().to_string()),
        orientation_kind,
        checkpoint_id: Some(request.checkpoint_id.clone()),
        related_task_list_id: task_context.and_then(|context| context.task_list_id.clone()),
        related_task_item_id: task_context.and_then(|context| context.active_item_id.clone()),
        related_docket_job_id,
        related_docket_task_id,
        evidence_refs: request
            .evidence_refs
            .iter()
            .map(|evidence| LedgerEvidenceRef {
                kind: evidence.kind.clone(),
                id: evidence.id.clone(),
            })
            .collect(),
        decision,
    })
}

fn row_to_ledger(row: sqlx::postgres::PgRow) -> LoopControlLedgerRow {
    LoopControlLedgerRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        turn_step_id: row.get("turn_step_id"),
        decision_id: row.get("decision_id"),
        decision_kind: row.get("decision_kind"),
        control_level: row.get("control_level"),
        reason: row.get("reason"),
        orientation_kind: row.get("orientation_kind"),
        checkpoint_id: row.get("checkpoint_id"),
        related_task_list_id: row.get("related_task_list_id"),
        related_task_item_id: row.get("related_task_item_id"),
        related_docket_job_id: row.get("related_docket_job_id"),
        related_docket_task_id: row.get("related_docket_task_id"),
        evidence_refs: row.get("evidence_refs"),
        decision: row.get("decision"),
        created_at: row.get("created_at"),
    }
}

pub async fn record_checkpoint_response(
    pool: &PgPool,
    input: CheckpointResponseInput,
) -> Result<CheckpointArtifactRow, DenError> {
    let response_json = serde_json::to_value(&input.response)
        .map_err(|err| DenError::System(format!("serialize checkpoint response: {err}")))?;
    let row = sqlx::query(
        r"
        UPDATE bear_run_checkpoints
        SET response = $3,
            validation_status = $4,
            updated_at = NOW()
        WHERE run_id = $1 AND checkpoint_id = $2
        RETURNING
            id, run_id, turn_step_id, checkpoint_id, reason, control_level, request,
            response, validation_status, visibility, replay_policy, related_task_list_id,
            related_task_item_id, related_docket_task_id, created_at, updated_at
        ",
    )
    .bind(&input.run_id)
    .bind(&input.checkpoint_id)
    .bind(response_json)
    .bind(input.validation_status.as_str())
    .fetch_optional(pool)
    .await?;

    row.map(row_to_checkpoint).ok_or_else(|| {
        DenError::NotFound(format!(
            "checkpoint artifact not found: run_id={} checkpoint_id={}",
            input.run_id, input.checkpoint_id
        ))
    })
}

pub async fn list_checkpoints_for_run(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<CheckpointArtifactRow>, DenError> {
    let rows = sqlx::query(
        r"
        SELECT
            id, run_id, turn_step_id, checkpoint_id, reason, control_level, request,
            response, validation_status, visibility, replay_policy, related_task_list_id,
            related_task_item_id, related_docket_task_id, created_at, updated_at
        FROM bear_run_checkpoints
        WHERE run_id = $1
        ORDER BY created_at ASC, checkpoint_id ASC
        ",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_checkpoint).collect())
}

pub async fn list_checkpoints_for_session(
    pool: &PgPool,
    bear_id: Uuid,
    session_id: &str,
    limit: i64,
) -> Result<Vec<CheckpointArtifactRow>, DenError> {
    let rows = sqlx::query(
        r"
        SELECT
            c.id, c.run_id, c.turn_step_id, c.checkpoint_id, c.reason, c.control_level,
            c.request, c.response, c.validation_status, c.visibility, c.replay_policy,
            c.related_task_list_id, c.related_task_item_id, c.related_docket_task_id,
            c.created_at, c.updated_at
        FROM bear_run_checkpoints c
        INNER JOIN turn_runs r ON r.run_id = c.run_id
        WHERE r.bear_id = $1 AND r.session_id = $2
        ORDER BY c.created_at DESC, c.checkpoint_id DESC
        LIMIT $3
        ",
    )
    .bind(bear_id)
    .bind(session_id)
    .bind(limit.max(1))
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_checkpoint).collect())
}

fn row_to_checkpoint(row: sqlx::postgres::PgRow) -> CheckpointArtifactRow {
    CheckpointArtifactRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        turn_step_id: row.get("turn_step_id"),
        checkpoint_id: row.get("checkpoint_id"),
        reason: row.get("reason"),
        control_level: row.get("control_level"),
        request: row.get("request"),
        response: row.get("response"),
        validation_status: row.get("validation_status"),
        visibility: row.get("visibility"),
        replay_policy: row.get("replay_policy"),
        related_task_list_id: row.get("related_task_list_id"),
        related_task_item_id: row.get("related_task_item_id"),
        related_docket_task_id: row.get("related_docket_task_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop::{
        CheckpointEvidenceRef, CheckpointField, CheckpointNextAction, CheckpointReason,
        CheckpointTaskContext, RuntimeCheckpointResponse,
    };
    use den_core::AgentLoopControlLevel;

    async fn seed_run(pool: &PgPool, run_id: &str) -> (Uuid, i32) {
        let suffix = Uuid::new_v4().simple().to_string();
        let username = format!("ckpt{}", &suffix[..12]);
        let email = format!("{username}@example.test");
        let (user_id,): (i32,) = sqlx::query_as(
            r"
            INSERT INTO users (email, username, display_name, passhash)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            ",
        )
        .bind(email)
        .bind(&username)
        .bind("Checkpoint Test")
        .bind("test-passhash")
        .fetch_one(pool)
        .await
        .expect("create user");
        let bear_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO bears (id, slug, name)
            VALUES ($1, $2, $3)
            ",
        )
        .bind(bear_id)
        .bind(format!("checkpoint-bear-{}", &suffix[..12]))
        .bind("Checkpoint Bear")
        .execute(pool)
        .await
        .expect("create bear");
        sqlx::query(
            r"
            INSERT INTO turn_runs (run_id, session_id, bear_id, user_id, state)
            VALUES ($1, $2, $3, $4, 'running')
            ",
        )
        .bind(run_id)
        .bind(format!("session-{run_id}"))
        .bind(bear_id)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("create run");
        (bear_id, user_id)
    }

    fn request(run_id: &str) -> RuntimeCheckpointRequest {
        RuntimeCheckpointRequest {
            checkpoint_id: "ckpt-1".to_string(),
            run_id: run_id.to_string(),
            reason: CheckpointReason::OverExploration,
            control_level: AgentLoopControlLevel::Careful,
            profile_fingerprint: Some("profile-test".to_string()),
            active_objective: Some("Find the failing path".to_string()),
            task_context: Some(CheckpointTaskContext {
                task_list_id: Some("list-1".to_string()),
                task_list_version: Some("2".to_string()),
                active_item_id: Some("item-1".to_string()),
                active_item_title: Some("Inspect runtime".to_string()),
                docket_job_id: None,
                docket_task_id: None,
            }),
            evidence_refs: vec![CheckpointEvidenceRef {
                kind: "tool_result".to_string(),
                id: "call-1".to_string(),
                summary: Some("Read runtime file".to_string()),
            }],
            required_fields: vec![CheckpointField::Learned],
        }
    }

    fn response() -> RuntimeCheckpointResponse {
        RuntimeCheckpointResponse {
            checkpoint_id: "ckpt-1".to_string(),
            active_objective: "Find the failing path".to_string(),
            summary: None,
            learned: vec!["The runtime parser is involved.".to_string()],
            remaining_uncertainty: vec![],
            more_exploration_justified: false,
            next_action: CheckpointNextAction::Validate,
            task_state_change_needed: None,
            evidence_refs: vec![],
            confidence: None,
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn records_grounding_probe_result_decision(pool: PgPool) {
        let run_id = format!("run-{}", Uuid::new_v4().simple());
        seed_run(&pool, &run_id).await;
        let mut input = non_empty_diff_grounding_probe(&run_id, "diff --git a/file b/file\n");
        input.tool_call_id = Some("call-grounded".to_string());

        let recorded = record_grounding_probe_result_decision(&pool, input)
            .await
            .expect("record grounding probe result");

        assert_eq!(recorded.decision_kind, "grounding_probe_result");
        assert_eq!(
            recorded.decision_id,
            "grounding_probe:generic.non_empty_diff"
        );
        assert_eq!(recorded.reason.as_deref(), Some("pass"));
        assert_eq!(recorded.evidence_refs[0]["kind"], "grounding_probe");
        assert_eq!(recorded.evidence_refs[0]["id"], "generic.non_empty_diff");
        assert_eq!(recorded.evidence_refs[1]["kind"], "tool_call");
        assert_eq!(recorded.evidence_refs[1]["id"], "call-grounded");
        assert_eq!(recorded.decision["surface_kind"], "repository");
        assert_eq!(recorded.decision["signal"], "pass");
        assert_eq!(recorded.decision["findings"][0]["code"], "diff_present");
        assert!(recorded.decision.get("diff").is_none());
        assert_eq!(
            latest_grounding_probe_signal_for_tool_call(&pool, &run_id, "call-grounded")
                .await
                .expect("latest tool grounding signal"),
            Some(GroundingProbeSignalKind::Pass)
        );
        assert_eq!(
            latest_grounding_probe_signal_for_tool_call(&pool, &run_id, "other-call")
                .await
                .expect("missing tool grounding signal"),
            None
        );
        assert_eq!(
            latest_grounding_probe_signal_for_run(&pool, &run_id)
                .await
                .expect("latest grounding signal"),
            Some(GroundingProbeSignalKind::Pass)
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn records_context_budget_pressure_decision(pool: PgPool) {
        let run_id = format!("run-{}", Uuid::new_v4().simple());
        seed_run(&pool, &run_id).await;
        let report = ContextBudgetReport {
            model: "test/model".to_string(),
            context_window: Some(100),
            max_output_tokens: Some(20),
            reserved_output_tokens: 10,
            estimated_input_tokens: 81,
            estimated_total_tokens: 91,
            estimate_precision: den_protocol::ContextBudgetEstimatePrecision::Approximate,
            near_budget: true,
            over_budget: false,
            calibration: None,
            components: vec![],
        };

        let recorded = record_context_budget_pressure_decision(
            &pool,
            &run_id,
            None,
            Some("oriented".to_string()),
            &report,
        )
        .await
        .expect("record context budget pressure")
        .expect("near budget writes ledger row");

        assert_eq!(recorded.decision_kind, "context_budget_pressure");
        assert_eq!(recorded.reason.as_deref(), Some("near_budget"));
        assert_eq!(recorded.orientation_kind.as_deref(), Some("oriented"));
        assert_eq!(recorded.evidence_refs[0]["kind"], "context_budget_report");
        assert_eq!(
            recorded.decision["estimated_total_tokens"],
            serde_json::json!(91)
        );
        assert_eq!(
            recorded.decision["action"],
            context_budget_pressure_action(ContextBudgetPressureLevel::NearBudget)
        );
        assert!(recorded.decision.get("components").is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn replay_turn_aggregate_handles_multi_decision_fixture(pool: PgPool) {
        let run_id = format!("run-{}", Uuid::new_v4().simple());
        seed_run(&pool, &run_id).await;
        let (turn_step_id,): (Uuid,) = sqlx::query_as(
            r"
            INSERT INTO turn_steps (run_id, step_index, state)
            VALUES ($1, 1, 'streaming_model')
            RETURNING id
            ",
        )
        .bind(&run_id)
        .fetch_one(&pool)
        .await
        .expect("create turn step");

        record_loop_control_decision(
            &pool,
            LoopControlLedgerInput {
                run_id: run_id.clone(),
                turn_step_id: Some(turn_step_id),
                decision_id: "multi:checkpoint".to_string(),
                decision_kind: LoopControlDecisionKind::CheckpointRequested,
                control_level: "careful".to_string(),
                reason: Some("over_exploration".to_string()),
                orientation_kind: Some("oriented".to_string()),
                checkpoint_id: Some("ckpt-multi".to_string()),
                related_task_list_id: Some("list-multi".to_string()),
                related_task_item_id: Some("item-multi".to_string()),
                related_docket_job_id: None,
                related_docket_task_id: None,
                evidence_refs: vec![LedgerEvidenceRef {
                    kind: "tool_result".to_string(),
                    id: "call-multi".to_string(),
                }],
                decision: serde_json::json!({ "active_objective_present": true }),
            },
        )
        .await
        .expect("record checkpoint decision");
        record_loop_control_decision(
            &pool,
            LoopControlLedgerInput {
                run_id: run_id.clone(),
                turn_step_id: Some(turn_step_id),
                decision_id: "multi:context_budget".to_string(),
                decision_kind: LoopControlDecisionKind::ContextBudgetPressure,
                control_level: "standard".to_string(),
                reason: Some("near_budget".to_string()),
                orientation_kind: Some("oriented".to_string()),
                checkpoint_id: None,
                related_task_list_id: Some("list-multi".to_string()),
                related_task_item_id: Some("item-multi".to_string()),
                related_docket_job_id: None,
                related_docket_task_id: None,
                evidence_refs: vec![LedgerEvidenceRef {
                    kind: "context_budget_report".to_string(),
                    id: "test/model:91:100".to_string(),
                }],
                decision: serde_json::json!({ "level": "near_budget" }),
            },
        )
        .await
        .expect("record context-budget decision");

        let ledger = list_loop_control_decisions_for_run(&pool, &run_id)
            .await
            .expect("list ledger decisions");
        let turns = aggregate_loop_control_replay_turns(&ledger).expect("aggregate replay turns");
        let expected_turns = vec![ExpectedLoopControlReplayTurn {
            run_id: run_id.clone(),
            turn_step_id: Some(turn_step_id),
            decision_count: 2,
            decision_kinds: vec![
                LoopControlDecisionKind::CheckpointRequested,
                LoopControlDecisionKind::ContextBudgetPressure,
            ],
            control_levels: vec!["careful".to_string(), "standard".to_string()],
            reasons: vec!["over_exploration".to_string(), "near_budget".to_string()],
            orientation_kinds: vec!["oriented".to_string()],
            checkpoint_ids: vec!["ckpt-multi".to_string()],
            evidence_refs: vec![
                LedgerEvidenceRef {
                    kind: "tool_result".to_string(),
                    id: "call-multi".to_string(),
                },
                LedgerEvidenceRef {
                    kind: "context_budget_report".to_string(),
                    id: "test/model:91:100".to_string(),
                },
            ],
        }];
        assert!(compare_loop_control_replay_turns(&turns, &expected_turns).is_empty());

        let profile = summarize_loop_control_replay_profile(&turns);
        let loaded_profile = summarize_loop_control_replay_profile_for_run(&pool, &run_id)
            .await
            .expect("summarize replay profile for run");
        assert_eq!(loaded_profile, profile);
        assert_eq!(profile.turn_count, 1);
        assert_eq!(profile.decision_count, 2);
        assert_eq!(
            profile.decision_kind_counts,
            vec![
                LoopControlReplayCount {
                    value: LoopControlDecisionKind::CheckpointRequested,
                    count: 1,
                },
                LoopControlReplayCount {
                    value: LoopControlDecisionKind::ContextBudgetPressure,
                    count: 1,
                },
            ]
        );
        assert_eq!(
            profile.control_level_counts,
            vec![
                LoopControlReplayCount {
                    value: "careful".to_string(),
                    count: 1,
                },
                LoopControlReplayCount {
                    value: "standard".to_string(),
                    count: 1,
                },
            ]
        );
        assert_eq!(
            profile.orientation_kind_counts,
            vec![LoopControlReplayCount {
                value: "oriented".to_string(),
                count: 1,
            }]
        );
        assert_eq!(
            profile.reason_counts,
            vec![
                LoopControlReplayCount {
                    value: "over_exploration".to_string(),
                    count: 1,
                },
                LoopControlReplayCount {
                    value: "near_budget".to_string(),
                    count: 1,
                },
            ]
        );

        let expected_profile = ExpectedLoopControlReplayProfileSummary {
            turn_count: 1,
            decision_count: 2,
            decision_kind_counts: vec![
                LoopControlReplayCount {
                    value: LoopControlDecisionKind::CheckpointRequested,
                    count: 1,
                },
                LoopControlReplayCount {
                    value: LoopControlDecisionKind::ContextBudgetPressure,
                    count: 1,
                },
            ],
            control_level_counts: vec![
                LoopControlReplayCount {
                    value: "careful".to_string(),
                    count: 1,
                },
                LoopControlReplayCount {
                    value: "standard".to_string(),
                    count: 1,
                },
            ],
            orientation_kind_counts: vec![LoopControlReplayCount {
                value: "oriented".to_string(),
                count: 1,
            }],
            reason_counts: vec![
                LoopControlReplayCount {
                    value: "over_exploration".to_string(),
                    count: 1,
                },
                LoopControlReplayCount {
                    value: "near_budget".to_string(),
                    count: 1,
                },
            ],
        };
        assert!(compare_loop_control_replay_profile(&profile, &expected_profile).is_none());
        let mut wrong_profile = expected_profile;
        wrong_profile.decision_count = 1;
        let mismatch = compare_loop_control_replay_profile(&profile, &wrong_profile)
            .expect("profile mismatch");
        assert_eq!(mismatch.observed, profile);
        assert_eq!(mismatch.expected.decision_count, 1);

        let mut wrong_turns = expected_turns;
        wrong_turns[0].reasons = vec!["rule_of_ko".to_string()];
        let mismatches = compare_loop_control_replay_turns(&turns, &wrong_turns);
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].observed.as_ref(), Some(&turns[0]));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn records_checkpoint_request_and_response(pool: PgPool) {
        let run_id = format!("run-{}", Uuid::new_v4().simple());
        let (bear_id, _) = seed_run(&pool, &run_id).await;

        let recorded = record_checkpoint_request(
            &pool,
            CheckpointArtifactInput {
                run_id: run_id.clone(),
                turn_step_id: None,
                orientation_kind: Some("focused".to_string()),
                request: request(&run_id),
                visibility: CheckpointVisibility::AuditOnly,
                replay_policy: CheckpointReplayPolicy::None,
            },
        )
        .await
        .expect("record request");

        assert_eq!(recorded.run_id, run_id);
        assert_eq!(recorded.checkpoint_id, "ckpt-1");
        assert_eq!(recorded.validation_status, "requested");
        assert_eq!(recorded.visibility, "audit_only");
        assert_eq!(recorded.replay_policy, "none");
        assert_eq!(recorded.related_task_list_id.as_deref(), Some("list-1"));
        assert_eq!(recorded.related_task_item_id.as_deref(), Some("item-1"));
        assert!(recorded.response.is_none());

        let updated = record_checkpoint_response(
            &pool,
            CheckpointResponseInput {
                run_id: run_id.clone(),
                checkpoint_id: "ckpt-1".to_string(),
                response: response(),
                validation_status: CheckpointValidationStatus::Valid,
            },
        )
        .await
        .expect("record response");
        assert_eq!(updated.validation_status, "valid");
        assert!(updated.response.is_some());

        let all = list_checkpoints_for_run(&pool, &run_id)
            .await
            .expect("list checkpoints");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].checkpoint_id, "ckpt-1");

        let ledger = list_loop_control_decisions_for_run(&pool, &run_id)
            .await
            .expect("list ledger decisions");
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger[0].decision_id, "checkpoint:ckpt-1");
        assert_eq!(ledger[0].decision_kind, "checkpoint_requested");
        assert_eq!(ledger[0].checkpoint_id.as_deref(), Some("ckpt-1"));
        assert_eq!(ledger[0].orientation_kind.as_deref(), Some("focused"));
        assert_eq!(ledger[0].reason.as_deref(), Some("over_exploration"));
        assert_eq!(ledger[0].related_task_list_id.as_deref(), Some("list-1"));
        assert_eq!(ledger[0].related_task_item_id.as_deref(), Some("item-1"));
        assert_eq!(ledger[0].evidence_refs[0]["id"], "call-1");
        assert!(ledger[0].evidence_refs[0].get("summary").is_none());
        assert_eq!(
            ledger[0].decision["active_objective_present"],
            serde_json::json!(true)
        );
        assert_eq!(
            ledger[0].decision["profile_fingerprint"],
            serde_json::json!("profile-test")
        );
        assert!(ledger[0].decision.get("active_objective").is_none());

        let observed = replay_loop_control_observations(&ledger).expect("replay observations");
        assert_eq!(observed.len(), 1);
        let expected = vec![LoopControlReplayObservation {
            run_id: run_id.clone(),
            turn_step_id: None,
            decision_id: "checkpoint:ckpt-1".to_string(),
            decision_kind: LoopControlDecisionKind::CheckpointRequested,
            control_level: "careful".to_string(),
            reason: Some("over_exploration".to_string()),
            orientation_kind: Some("focused".to_string()),
            checkpoint_id: Some("ckpt-1".to_string()),
            related_task_list_id: Some("list-1".to_string()),
            related_task_item_id: Some("item-1".to_string()),
            related_docket_job_id: None,
            related_docket_task_id: None,
            evidence_refs: vec![LedgerEvidenceRef {
                kind: "tool_result".to_string(),
                id: "call-1".to_string(),
            }],
        }];
        assert!(compare_loop_control_replay(&observed, &expected).is_empty());

        let turns = aggregate_loop_control_replay_turns(&ledger).expect("aggregate replay turns");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].run_id, run_id);
        assert_eq!(turns[0].turn_step_id, None);
        assert_eq!(turns[0].decision_count, 1);
        assert_eq!(turns[0].decision_ids, vec!["checkpoint:ckpt-1"]);
        assert_eq!(
            turns[0].decision_kinds,
            vec![LoopControlDecisionKind::CheckpointRequested]
        );
        assert_eq!(turns[0].control_levels, vec!["careful"]);
        assert_eq!(turns[0].orientation_kinds, vec!["focused"]);
        assert_eq!(turns[0].checkpoint_ids, vec!["ckpt-1"]);
        assert_eq!(turns[0].related_task_list_ids, vec!["list-1"]);
        assert_eq!(turns[0].related_task_item_ids, vec!["item-1"]);
        assert_eq!(
            turns[0].evidence_refs,
            vec![LedgerEvidenceRef {
                kind: "tool_result".to_string(),
                id: "call-1".to_string(),
            }]
        );

        let expected_turns = vec![ExpectedLoopControlReplayTurn {
            run_id: run_id.clone(),
            turn_step_id: None,
            decision_count: 1,
            decision_kinds: vec![LoopControlDecisionKind::CheckpointRequested],
            control_levels: vec!["careful".to_string()],
            reasons: vec!["over_exploration".to_string()],
            orientation_kinds: vec!["focused".to_string()],
            checkpoint_ids: vec!["ckpt-1".to_string()],
            evidence_refs: vec![LedgerEvidenceRef {
                kind: "tool_result".to_string(),
                id: "call-1".to_string(),
            }],
        }];
        assert!(compare_loop_control_replay_turns(&turns, &expected_turns).is_empty());

        let mismatches = compare_loop_control_replay_turns(&turns, &[]);
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].observed.as_ref(), Some(&turns[0]));
        assert_eq!(mismatches[0].expected, None);

        let by_session =
            list_checkpoints_for_session(&pool, bear_id, &format!("session-{run_id}"), 10)
                .await
                .expect("list checkpoints by session");
        assert_eq!(by_session.len(), 1);
        assert_eq!(by_session[0].checkpoint_id, "ckpt-1");
    }
}
