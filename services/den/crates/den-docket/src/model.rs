//! Docket domain types and compatibility projection shapes.
//!
//! ADR-0034 relational jobs/tasks are the canonical storage model. The
//! `TaskList*` and task-list projection types remain as API/projection shapes
//! for existing tool contracts; they should not imply an active
//! `bear_task_lists` persistence path.

use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::FromRow;
use std::fmt::{self, Write as _};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskListVisibility {
    PrivateToProfile,
    SameUser,
    BearVisible,
    HandoffRequested,
}

impl TaskListVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrivateToProfile => "private_to_profile",
            Self::SameUser => "same_user",
            Self::BearVisible => "bear_visible",
            Self::HandoffRequested => "handoff_requested",
        }
    }

    pub fn parse(value: &str) -> Result<Self, DenError> {
        match value.trim() {
            "private_to_profile" => Ok(Self::PrivateToProfile),
            "same_user" => Ok(Self::SameUser),
            "bear_visible" => Ok(Self::BearVisible),
            "handoff_requested" => Ok(Self::HandoffRequested),
            other => Err(DenError::Parsing(format!(
                "unknown task-list visibility: {other}"
            ))),
        }
    }
}

impl fmt::Display for TaskListVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskListStatus {
    Active,
    Blocked,
    Completed,
    Cancelled,
    Archived,
}

impl TaskListStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Archived => "archived",
        }
    }

    pub fn parse(value: &str) -> Result<Self, DenError> {
        match value.trim() {
            "active" => Ok(Self::Active),
            "blocked" => Ok(Self::Blocked),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "archived" => Ok(Self::Archived),
            other => Err(DenError::Parsing(format!(
                "unknown task-list status: {other}"
            ))),
        }
    }
}

impl fmt::Display for TaskListStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskListItemStatus {
    Pending,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}

impl TaskListItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for TaskListItemStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskListUpdateItem {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub status: TaskListItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskListUpdate {
    pub title: String,
    pub summary: String,
    pub visibility: TaskListVisibility,
    pub status: TaskListStatus,
    pub items: Vec<TaskListUpdateItem>,
    pub workspace_context: serde_json::Value,
}

impl<'de> Deserialize<'de> for TaskListUpdate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawTaskListUpdate {
            title: String,
            #[serde(default)]
            summary: String,
            visibility: TaskListVisibility,
            status: TaskListStatus,
            #[serde(default)]
            items: Vec<TaskListUpdateItem>,
            #[serde(default = "default_json_object")]
            workspace_context: serde_json::Value,
        }

        let mut raw = RawTaskListUpdate::deserialize(deserializer)?;
        normalize_task_list_item_ids(&mut raw.items);
        Ok(Self {
            title: raw.title,
            summary: raw.summary,
            visibility: raw.visibility,
            status: raw.status,
            items: raw.items,
            workspace_context: raw.workspace_context,
        })
    }
}

fn default_json_object() -> serde_json::Value {
    serde_json::json!({})
}

pub fn normalize_task_list_item_ids(items: &mut [TaskListUpdateItem]) {
    let mut generated_ids = std::collections::HashSet::new();
    for item in items {
        let trimmed = item.id.trim();
        if trimmed.is_empty() {
            let mut generated = generated_task_list_item_id(item, None);
            if !generated_ids.insert(generated.clone()) {
                let mut ordinal = 2_u32;
                loop {
                    generated = generated_task_list_item_id(item, Some(ordinal));
                    if generated_ids.insert(generated.clone()) {
                        break;
                    }
                    ordinal = ordinal.saturating_add(1);
                }
            }
            item.id = generated;
        } else if trimmed.len() != item.id.len() {
            item.id = trimmed.to_string();
        }
    }
}

fn generated_task_list_item_id(item: &TaskListUpdateItem, ordinal: Option<u32>) -> String {
    let mut seed = format!(
        "{}\n{}\n{}",
        item.title.trim(),
        item.summary.as_deref().unwrap_or("").trim(),
        item.status.as_str()
    );
    if let Some(ordinal) = ordinal {
        let _ = write!(seed, "\n{ordinal}");
    }
    let prefix = slug_prefix(&item.title).unwrap_or_else(|| "item".to_string());
    format!("{}_{:06x}", prefix, fnv1a64(seed.as_bytes()) & 0x00ff_ffff)
}

fn slug_prefix(value: &str) -> Option<String> {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for ch in value.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('_');
            last_was_separator = true;
        }
        if slug.len() >= 32 {
            break;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    (!slug.is_empty()).then_some(slug)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskListLocalProjection {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub title: String,
    pub summary: String,
    pub owner_profile: String,
    pub visibility: String,
    pub status: String,
    pub version: i32,
    pub items: Vec<TaskListUpdateItem>,
    pub current_item: Option<TaskListUpdateItem>,
    pub source_conversation_id: Option<String>,
    pub source_client_session_id: Option<String>,
    pub handoff_intent_path: Option<String>,
    pub handoff_task_id: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskListSyncState {
    LocalOnly,
    CheckedOut,
    Clean,
    Dirty,
    Syncing,
    Synced,
    Conflict,
    ReviewRequired,
}

impl TaskListSyncState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::CheckedOut => "checked_out",
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::Syncing => "syncing",
            Self::Synced => "synced",
            Self::Conflict => "conflict",
            Self::ReviewRequired => "review_required",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskListSourceRef {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docket_job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docket_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
}

impl TaskListSourceRef {
    pub fn local(refs: Vec<String>) -> Self {
        Self {
            kind: "local".to_string(),
            docket_job_id: None,
            docket_task_id: None,
            refs,
        }
    }

    pub fn docket_job(job_id: String, refs: Vec<String>) -> Self {
        Self {
            kind: "docket_job".to_string(),
            docket_job_id: Some(job_id),
            docket_task_id: None,
            refs,
        }
    }

    pub fn docket_task(job_id: Option<String>, task_id: String, refs: Vec<String>) -> Self {
        Self {
            kind: "docket_task".to_string(),
            docket_job_id: job_id,
            docket_task_id: Some(task_id),
            refs,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskListItem {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub status: TaskListItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    pub source_ref: TaskListSourceRef,
    pub sync_state: TaskListSyncState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskListProjection {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub title: String,
    pub summary: String,
    pub owner_profile: String,
    pub visibility: String,
    pub status: String,
    pub version: i32,
    pub source_ref: TaskListSourceRef,
    pub items: Vec<TaskListItem>,
    pub current_item: Option<TaskListItem>,
    pub source_conversation_id: Option<String>,
    pub source_client_session_id: Option<String>,
    pub handoff_intent_path: Option<String>,
    pub handoff_task_id: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskListValidationError {
    EmptyTitle,
    EmptyItemId,
    EmptyItemTitle { item_id: String },
    MultipleInProgressItems,
    BlockedItemMissingReason { item_id: String },
}

impl fmt::Display for TaskListValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTitle => f.write_str("task-list title must not be empty"),
            Self::EmptyItemId => f.write_str("task-list item id must not be empty"),
            Self::EmptyItemTitle { item_id } => {
                write!(f, "task-list item `{item_id}` title must not be empty")
            }
            Self::MultipleInProgressItems => {
                f.write_str("task list may have at most one in_progress item")
            }
            Self::BlockedItemMissingReason { item_id } => {
                write!(
                    f,
                    "blocked task-list item `{item_id}` must include blocked_reason"
                )
            }
        }
    }
}

impl std::error::Error for TaskListValidationError {}

impl From<TaskListValidationError> for DenError {
    fn from(err: TaskListValidationError) -> Self {
        DenError::ValidationError(err.to_string())
    }
}

fn parse_docket_source_refs(refs: &[String]) -> Option<TaskListSourceRef> {
    let mut job_id = None;
    let mut task_id = None;
    for raw in refs {
        let value = raw.trim();
        if let Some(rest) = value.strip_prefix("docket_job:") {
            if !rest.trim().is_empty() {
                job_id = Some(rest.trim().to_string());
            }
        } else if let Some(rest) = value.strip_prefix("docket_task:") {
            if !rest.trim().is_empty() {
                task_id = Some(rest.trim().to_string());
            }
        }
    }
    task_id.map(|task_id| TaskListSourceRef::docket_task(job_id, task_id, refs.to_vec()))
}

pub fn task_list_item_from_update_item(item: &TaskListUpdateItem) -> TaskListItem {
    let source_ref = parse_docket_source_refs(&item.source_refs)
        .unwrap_or_else(|| TaskListSourceRef::local(item.source_refs.clone()));
    let sync_state = if source_ref.kind == "docket_task" {
        TaskListSyncState::CheckedOut
    } else {
        TaskListSyncState::LocalOnly
    };
    TaskListItem {
        id: item.id.clone(),
        title: item.title.clone(),
        summary: item.summary.clone(),
        status: item.status,
        blocked_reason: item.blocked_reason.clone(),
        source_ref,
        sync_state,
    }
}

pub fn task_list_projection_from_local(plan: &TaskListLocalProjection) -> TaskListProjection {
    let items = plan
        .items
        .iter()
        .map(task_list_item_from_update_item)
        .collect::<Vec<_>>();
    let current_item = plan
        .current_item
        .as_ref()
        .map(task_list_item_from_update_item);
    TaskListProjection {
        id: plan.id,
        bear_id: plan.bear_id,
        title: plan.title.clone(),
        summary: plan.summary.clone(),
        owner_profile: plan.owner_profile.clone(),
        visibility: plan.visibility.clone(),
        status: plan.status.clone(),
        version: plan.version,
        source_ref: TaskListSourceRef::local(vec![format!("task_list:{}", plan.id)]),
        items,
        current_item,
        source_conversation_id: plan.source_conversation_id.clone(),
        source_client_session_id: plan.source_client_session_id.clone(),
        handoff_intent_path: plan.handoff_intent_path.clone(),
        handoff_task_id: plan.handoff_task_id.clone(),
        created_at: plan.created_at,
        updated_at: plan.updated_at,
    }
}

impl TaskListLocalProjection {
    pub fn to_task_list_projection(&self) -> TaskListProjection {
        task_list_projection_from_local(self)
    }
}

#[derive(Debug, Clone)]
pub enum TaskListCheckoutSource {
    DocketJob {
        job_id: Uuid,
        parent_task_id: Option<Uuid>,
    },
    LocalProjection(Box<TaskListProjection>),
}

#[derive(Debug, Clone)]
pub struct TaskListCheckoutRequest {
    pub source: TaskListCheckoutSource,
}

#[derive(Debug, Clone)]
pub struct TaskListSyncRequest {
    pub task_list: TaskListProjection,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskListSyncOutcome {
    pub task_list: TaskListProjection,
    pub applied: bool,
    pub review_required: bool,
    pub conflicts: Vec<String>,
    pub message: String,
}

impl TaskListSyncOutcome {
    pub fn applied(task_list: TaskListProjection, message: impl Into<String>) -> Self {
        Self {
            task_list,
            applied: true,
            review_required: false,
            conflicts: Vec::new(),
            message: message.into(),
        }
    }

    pub fn conflicts(
        task_list: TaskListProjection,
        conflicts: Vec<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            task_list,
            applied: false,
            review_required: false,
            conflicts,
            message: message.into(),
        }
    }

    pub fn review_required(task_list: TaskListProjection, message: impl Into<String>) -> Self {
        Self {
            task_list,
            applied: false,
            review_required: true,
            conflicts: Vec::new(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskListHandoffRequest {
    pub task_list: TaskListProjection,
    pub item_ids: Vec<String>,
    pub title: String,
    pub summary: String,
    pub requested_outcome: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskListHandoffOutcome {
    pub accepted: bool,
    pub review_required: bool,
    pub message: String,
    pub task_list_id: Uuid,
    pub item_ids: Vec<String>,
}

impl TaskListHandoffOutcome {
    pub fn review_required(request: &TaskListHandoffRequest, message: impl Into<String>) -> Self {
        Self {
            accepted: true,
            review_required: true,
            message: message.into(),
            task_list_id: request.task_list.id,
            item_ids: request.item_ids.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketJobStatus {
    Draft,
    Ready,
    Running,
    Blocked,
    Completed,
    Cancelled,
    Archived,
}

impl DocketJobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Archived => "archived",
        }
    }
}

impl fmt::Display for DocketJobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Explicit caller intent when a new job has the same normalized goal and
/// work surface as an active job. This is deliberately not a fuzzy goal
/// similarity heuristic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DocketJobOverlapResolution {
    #[default]
    Reject,
    Independent,
    Supersede,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketCommitPolicy {
    None,
    PerTask,
    PerJob,
}

impl DocketCommitPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PerTask => "per_task",
            Self::PerJob => "per_job",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketCriterionKind {
    Narrative,
    Command,
    CheckRef,
}

impl DocketCriterionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Narrative => "narrative",
            Self::Command => "command",
            Self::CheckRef => "check_ref",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketRunTrigger {
    Manual,
    Scheduled,
    Event,
}

impl DocketRunTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
            Self::Event => "event",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketRunState {
    Dispatched,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl DocketRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatched => "dispatched",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketTaskKind {
    Execution,
    Investigation,
    Decision,
}

impl DocketTaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::Investigation => "investigation",
            Self::Decision => "decision",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketTaskScope {
    Template,
    Run,
}

impl DocketTaskScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Template => "template",
            Self::Run => "run",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketTaskDifficulty {
    Trivial,
    Moderate,
    Hard,
    Unknown,
}

impl DocketTaskDifficulty {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trivial => "trivial",
            Self::Moderate => "moderate",
            Self::Hard => "hard",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketEffortHint {
    Low,
    Medium,
    High,
}

impl DocketEffortHint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    Inline,
    Scoped,
    Delegated,
    #[default]
    Auto,
}

impl RoutingStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Scoped => "scoped",
            Self::Delegated => "delegated",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultRollupPolicy {
    SummaryToParent,
    None,
}

impl ResultRollupPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SummaryToParent => "summary_to_parent",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketTaskStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
    Cancelled,
}

impl DocketTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocketCriterionStatus {
    Unmet,
    Met,
    Waived,
}

impl DocketCriterionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unmet => "unmet",
            Self::Met => "met",
            Self::Waived => "waived",
        }
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct DocketJobRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub created_by_user_id: i32,
    pub created_by_role: String,
    pub goal: String,
    /// Canonical managed work surface this job runs on (`work_surfaces.id`).
    pub work_surface_id: Option<Uuid>,
    pub commit_policy: Option<String>,
    /// Upstream branch this job's work runs publish to (set on first
    /// pushable dispatch when absent; default `den/job-<short-id>`).
    pub work_branch: Option<String>,
    pub status: String,
    pub visibility: String,
    pub source_conversation_id: Option<String>,
    pub objective_kind: Option<String>,
    /// The active job explicitly replaced by this job, if any. This preserves
    /// ownership/history without guessing semantic equivalence from prose.
    pub supersedes_job_id: Option<Uuid>,
    pub current_run_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct DocketJobCriterionRow {
    pub id: Uuid,
    pub job_id: Uuid,
    pub kind: String,
    pub description: String,
    pub spec: Option<Json<serde_json::Value>>,
    pub sibling_order: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct DocketJobRunRow {
    pub id: Uuid,
    pub job_id: Uuid,
    pub trigger: String,
    pub schedule_ref: Option<String>,
    pub state: String,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub outcome: Option<Json<serde_json::Value>>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct DocketTaskRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub job_id: Option<Uuid>,
    pub session_anchor_id: Option<Uuid>,
    pub parent_task_id: Option<Uuid>,
    pub sibling_order: i32,
    pub kind: String,
    pub scope: String,
    pub title: String,
    pub body: String,
    pub completion_criteria: Json<Vec<String>>,
    pub difficulty: Option<String>,
    pub effort_hint: Option<String>,
    pub routing_strategy: String,
    pub expected_context_size: Option<i32>,
    pub result_rollup_policy: Option<String>,
    pub created_by_role: String,
    pub created_by_user_id: Option<i32>,
    pub created_by_agent_id: Option<String>,
    pub created_in_run_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct DocketTaskRunStateRow {
    pub run_id: Uuid,
    pub task_id: Uuid,
    pub status: String,
    pub result_refs: Option<Json<serde_json::Value>>,
    pub result_summary: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct DocketCriterionStateRow {
    pub run_id: Uuid,
    pub criterion_id: Uuid,
    pub status: String,
    pub evaluated_at: Option<OffsetDateTime>,
    pub evidence: Option<Json<serde_json::Value>>,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocketJobProjection {
    pub job: DocketJobRow,
    pub current_run: Option<DocketJobRunRow>,
    pub criteria: Vec<DocketJobCriterionRow>,
    pub criteria_states: Vec<DocketCriterionStateRow>,
    pub tasks: Vec<DocketTaskRow>,
    pub task_states: Vec<DocketTaskRunStateRow>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocketCountByStatus {
    pub pending: usize,
    pub in_progress: usize,
    pub done: usize,
    pub blocked: usize,
    pub cancelled: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocketCriteriaCountByStatus {
    pub unmet: usize,
    pub met: usize,
    pub waived: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocketJobStatusReport {
    pub job_id: Uuid,
    pub run_id: Option<Uuid>,
    pub job_status: String,
    pub run_state: Option<String>,
    pub current_task_id: Option<Uuid>,
    pub current_task_title: Option<String>,
    pub task_counts: DocketCountByStatus,
    pub criteria_counts: DocketCriteriaCountByStatus,
    pub tasks_complete: bool,
    pub criteria_complete: bool,
    pub next_action: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DocketJobCriterionInput {
    #[serde(default = "default_criterion_kind")]
    pub kind: DocketCriterionKind,
    pub description: String,
    #[serde(default)]
    pub spec: Option<serde_json::Value>,
    #[serde(default)]
    pub sibling_order: i32,
}

fn default_criterion_kind() -> DocketCriterionKind {
    DocketCriterionKind::Narrative
}

#[derive(Debug, Clone, Deserialize)]
pub struct DocketTaskInput {
    #[serde(default)]
    pub client_key: Option<String>,
    #[serde(default)]
    pub parent_client_key: Option<String>,
    #[serde(default)]
    pub parent_task_id: Option<Uuid>,
    #[serde(default)]
    pub sibling_order: i32,
    #[serde(default = "default_task_kind")]
    pub kind: DocketTaskKind,
    #[serde(default = "default_task_scope")]
    pub scope: DocketTaskScope,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub completion_criteria: Vec<String>,
    #[serde(default)]
    pub difficulty: Option<DocketTaskDifficulty>,
    #[serde(default)]
    pub effort_hint: Option<DocketEffortHint>,
    #[serde(default)]
    pub routing_strategy: RoutingStrategy,
    #[serde(default)]
    pub expected_context_size: Option<i32>,
    #[serde(default)]
    pub result_rollup_policy: Option<ResultRollupPolicy>,
}

fn default_task_kind() -> DocketTaskKind {
    DocketTaskKind::Execution
}

fn default_task_scope() -> DocketTaskScope {
    DocketTaskScope::Template
}

#[derive(Debug, Clone)]
pub struct DocketJobCreate {
    pub bear_id: Uuid,
    pub created_by_user_id: i32,
    pub created_by_role: String,
    pub goal: String,
    /// Managed surface id, after checking the bear's assignment.
    pub work_surface_id: Option<Uuid>,
    pub commit_policy: Option<DocketCommitPolicy>,
    /// Explicit upstream branch for work-run publishing; generated
    /// (`den/job-<short-id>`) on first pushable dispatch when absent.
    pub work_branch: Option<String>,
    pub status: DocketJobStatus,
    pub visibility: TaskListVisibility,
    pub source_conversation_id: Option<String>,
    pub objective_kind: Option<String>,
    /// Explicitly replace this active job. Required only when
    /// `overlap_resolution` is `supersede`.
    pub supersedes_job_id: Option<Uuid>,
    pub overlap_resolution: DocketJobOverlapResolution,
    pub criteria: Vec<DocketJobCriterionInput>,
    pub tasks: Vec<DocketTaskInput>,
}

#[derive(Debug, Clone, Default)]
pub struct DocketJobListFilter {
    pub statuses: Option<Vec<DocketJobStatus>>,
    pub include_cancelled: bool,
    pub include_archived: bool,
    pub source_conversation_id: Option<String>,
    pub limit: i64,
}

#[derive(Debug, Clone)]
pub struct DocketJobUpdate {
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub actor_role: BearProfile,
    pub actor_user_id: Option<i32>,
    pub actor_agent_id: Option<String>,
    pub goal: Option<String>,
    /// Replace the managed surface binding. Work jobs cannot clear it.
    pub work_surface_id: Option<Option<Uuid>>,
    pub commit_policy: Option<Option<DocketCommitPolicy>>,
    /// Explicit publish branch; `Some(None)` clears it so the next pushable
    /// dispatch can generate the canonical `den/job-<id>` branch.
    pub work_branch: Option<Option<String>>,
    pub status: Option<DocketJobStatus>,
    pub visibility: Option<TaskListVisibility>,
}

#[derive(Debug, Clone)]
pub struct DocketCriterionStateUpdate {
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub criterion_id: Uuid,
    pub status: DocketCriterionStatus,
    pub evidence: Option<serde_json::Value>,
    pub actor_role: BearProfile,
    pub actor_user_id: Option<i32>,
    pub actor_agent_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DocketJobExecuteRequest {
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub actor_role: BearProfile,
    pub actor_user_id: Option<i32>,
    pub actor_agent_id: Option<String>,
    pub session_id: Option<String>,
    pub source_conversation_id: Option<String>,
    pub source_client_session_id: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct DocketExecutionSessionRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub owner_profile: String,
    pub session_id: String,
    pub source_conversation_id: Option<String>,
    pub source_client_session_id: Option<String>,
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub task_id: Option<Uuid>,
    pub state: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default)]
pub struct DocketExecutionLookup {
    pub session_id: Option<String>,
    pub source_conversation_id: Option<String>,
    pub source_client_session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DocketExecutionSessionUpsert {
    pub bear_id: Uuid,
    pub owner_profile: BearProfile,
    pub session_id: String,
    pub source_conversation_id: Option<String>,
    pub source_client_session_id: Option<String>,
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub task_id: Option<Uuid>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocketJobExecuteOutcome {
    pub job: DocketJobProjection,
    pub selected_task_id: Option<Uuid>,
    pub completed: bool,
    pub blocked: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct DocketTaskCreate {
    pub bear_id: Uuid,
    pub job_id: Option<Uuid>,
    pub session_anchor_id: Option<Uuid>,
    pub parent_task_id: Option<Uuid>,
    pub sibling_order: i32,
    pub kind: DocketTaskKind,
    pub scope: DocketTaskScope,
    pub title: String,
    pub body: String,
    pub completion_criteria: Vec<String>,
    pub difficulty: Option<DocketTaskDifficulty>,
    pub effort_hint: Option<DocketEffortHint>,
    pub routing_strategy: RoutingStrategy,
    pub expected_context_size: Option<i32>,
    pub result_rollup_policy: Option<ResultRollupPolicy>,
    pub created_by_role: String,
    pub created_by_user_id: Option<i32>,
    pub created_by_agent_id: Option<String>,
    pub created_in_run_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct DocketTaskListFilter {
    pub job_id: Option<Uuid>,
    pub session_anchor_id: Option<Uuid>,
    pub parent_task_id: Option<Uuid>,
    pub include_descendants: bool,
    pub limit: i64,
}

#[derive(Debug, Clone)]
pub struct DocketTaskRunStateUpdate {
    pub run_id: Uuid,
    pub status: DocketTaskStatus,
    pub result_refs: Option<serde_json::Value>,
    pub result_summary: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DocketTaskDefinitionPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub completion_criteria: Option<Vec<String>>,
    pub parent_task_id: Option<Option<Uuid>>,
    pub sibling_order: Option<i32>,
    pub kind: Option<DocketTaskKind>,
    pub scope: Option<DocketTaskScope>,
    pub difficulty: Option<Option<DocketTaskDifficulty>>,
    pub effort_hint: Option<Option<DocketEffortHint>>,
    pub routing_strategy: Option<RoutingStrategy>,
    pub expected_context_size: Option<Option<i32>>,
    pub result_rollup_policy: Option<Option<ResultRollupPolicy>>,
}

#[derive(Debug, Clone)]
pub struct DocketTaskUpdate {
    pub bear_id: Uuid,
    pub job_id: Option<Uuid>,
    pub task_id: Uuid,
    pub actor_role: BearProfile,
    pub actor_user_id: Option<i32>,
    pub actor_agent_id: Option<String>,
    pub definition: DocketTaskDefinitionPatch,
    pub run_state: Option<DocketTaskRunStateUpdate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocketTaskProjection {
    pub task: DocketTaskRow,
    pub run_state: Option<DocketTaskRunStateRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocketValidationError {
    EmptyGoal,
    MissingWorkSurface,
    MismatchedWorkSurfaceBinding,
    InvalidJobCreatorRole { role: String },
    EmptyCriterionDescription,
    EmptyTaskTitle,
    EmptyTaskBody,
    EmptyTaskCompletionCriteria,
    EmptyTaskCompletionCriterion,
    TaskMissingAnchor,
    DuplicateTaskClientKey { client_key: String },
    MissingParentClientKey { client_key: String },
    SupersedeRequiresPredecessor,
    SupersedeRequiresMatchingActiveJob { job_id: Uuid },
    ActiveJobOverlap { job_id: Uuid },
}

impl fmt::Display for DocketValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyGoal => f.write_str("Docket job goal must not be empty"),
            Self::MissingWorkSurface => {
                f.write_str("Docket work job requires a managed work surface")
            }
            Self::MismatchedWorkSurfaceBinding => f.write_str(
                "Docket work job surface name and managed surface ID must be set together",
            ),
            Self::InvalidJobCreatorRole { role } => {
                write!(
                    f,
                    "Docket jobs must be human-created via chat, pair, or ui, not `{role}`"
                )
            }
            Self::EmptyCriterionDescription => {
                f.write_str("Docket job criterion description must not be empty")
            }
            Self::EmptyTaskTitle => f.write_str("Docket task title must not be empty"),
            Self::EmptyTaskBody => f.write_str("Docket task body must not be empty"),
            Self::EmptyTaskCompletionCriteria => {
                f.write_str("Docket task completion_criteria must include at least one criterion")
            }
            Self::EmptyTaskCompletionCriterion => {
                f.write_str("Docket task completion_criteria must not include blank criteria")
            }
            Self::TaskMissingAnchor => {
                f.write_str("Docket task must be anchored to either a job or an client session")
            }
            Self::DuplicateTaskClientKey { client_key } => {
                write!(f, "Docket task client_key `{client_key}` is duplicated")
            }
            Self::MissingParentClientKey { client_key } => {
                write!(
                    f,
                    "Docket task parent_client_key `{client_key}` does not exist"
                )
            }
            Self::SupersedeRequiresPredecessor => {
                f.write_str("Docket job overlap resolution `supersede` requires supersedes_job_id")
            }
            Self::SupersedeRequiresMatchingActiveJob { job_id } => {
                write!(
                    f,
                    "Docket job `{job_id}` is not an active matching predecessor"
                )
            }
            Self::ActiveJobOverlap { job_id } => {
                write!(
                    f,
                    "an active Docket job already owns this goal and work surface: {job_id}"
                )
            }
        }
    }
}

impl std::error::Error for DocketValidationError {}

impl From<DocketValidationError> for DenError {
    fn from(err: DocketValidationError) -> Self {
        DenError::ValidationError(err.to_string())
    }
}

pub fn docket_job_status_report(projection: &DocketJobProjection) -> DocketJobStatusReport {
    let mut task_counts = DocketCountByStatus {
        pending: 0,
        in_progress: 0,
        done: 0,
        blocked: 0,
        cancelled: 0,
    };
    let task_states_by_id = projection
        .task_states
        .iter()
        .map(|state| (state.task_id, state.status.as_str()))
        .collect::<std::collections::HashMap<_, _>>();
    let current_task = projection.tasks.iter().find(|task| {
        task_states_by_id
            .get(&task.id)
            .is_some_and(|status| *status == "in_progress")
    });
    for task in &projection.tasks {
        match task_states_by_id
            .get(&task.id)
            .copied()
            .unwrap_or("pending")
        {
            "in_progress" => task_counts.in_progress += 1,
            "done" => task_counts.done += 1,
            "blocked" => task_counts.blocked += 1,
            "cancelled" => task_counts.cancelled += 1,
            _ => task_counts.pending += 1,
        }
    }

    let mut criteria_counts = DocketCriteriaCountByStatus {
        unmet: 0,
        met: 0,
        waived: 0,
    };
    let criteria_states_by_id = projection
        .criteria_states
        .iter()
        .map(|state| (state.criterion_id, state.status.as_str()))
        .collect::<std::collections::HashMap<_, _>>();
    for criterion in &projection.criteria {
        match criteria_states_by_id
            .get(&criterion.id)
            .copied()
            .unwrap_or("unmet")
        {
            "met" => criteria_counts.met += 1,
            "waived" => criteria_counts.waived += 1,
            _ => criteria_counts.unmet += 1,
        }
    }

    let tasks_complete = task_counts.pending == 0
        && task_counts.in_progress == 0
        && task_counts.blocked == 0
        && task_counts.done + task_counts.cancelled == projection.tasks.len();
    let criteria_complete = projection.criteria.is_empty()
        || criteria_counts.unmet == 0
            && criteria_counts.met + criteria_counts.waived == projection.criteria.len();
    let next_action = if task_counts.blocked > 0 || projection.job.status == "blocked" {
        "resolve_blocked_task_or_criterion".to_string()
    } else if task_counts.in_progress > 0 {
        "continue_current_task".to_string()
    } else if task_counts.pending > 0 {
        "execute_job_to_select_next_task".to_string()
    } else if !criteria_complete {
        "evaluate_remaining_criteria".to_string()
    } else if projection.job.status != "completed" {
        "execute_job_to_complete".to_string()
    } else {
        "done".to_string()
    };

    DocketJobStatusReport {
        job_id: projection.job.id,
        run_id: projection.current_run.as_ref().map(|run| run.id),
        job_status: projection.job.status.clone(),
        run_state: projection.current_run.as_ref().map(|run| run.state.clone()),
        current_task_id: current_task.map(|task| task.id),
        current_task_title: current_task.map(|task| task.title.clone()),
        task_counts,
        criteria_counts,
        tasks_complete,
        criteria_complete,
        next_action,
    }
}

pub fn active_task_parent_id(projection: &DocketJobProjection) -> Option<Uuid> {
    let active_task_id = projection
        .task_states
        .iter()
        .find(|state| state.status == "in_progress")?
        .task_id;
    projection
        .tasks
        .iter()
        .find(|task| task.id == active_task_id)
        .and_then(|task| task.parent_task_id)
}

pub fn task_list_projection_from_docket_job(
    projection: &DocketJobProjection,
    parent_task_id: Option<Uuid>,
) -> TaskListProjection {
    let states_by_task_id = projection
        .task_states
        .iter()
        .map(|state| (state.task_id, state))
        .collect::<std::collections::HashMap<_, _>>();
    let mut tasks = projection
        .tasks
        .iter()
        .filter(|task| task.parent_task_id == parent_task_id)
        .collect::<Vec<_>>();
    // Keep projection order deterministic at the task-list boundary. Upstream DB
    // queries usually order by sibling_order, but ACP "agent plan" rendering
    // should not depend on every caller preserving that order.
    tasks.sort_by_key(|task| (task.sibling_order, task.created_at, task.id));
    let items = tasks
        .into_iter()
        .map(|task| task_list_item_from_docket_task(task, states_by_task_id.get(&task.id).copied()))
        .collect::<Vec<_>>();
    let current_item = current_task_list_item(&items).cloned();

    TaskListProjection {
        id: projection.job.id,
        bear_id: projection.job.bear_id,
        title: projection.job.goal.clone(),
        summary: "Docket work job checkout".to_string(),
        owner_profile: projection.job.created_by_role.clone(),
        visibility: projection.job.visibility.clone(),
        status: projection.job.status.clone(),
        version: 1,
        source_ref: TaskListSourceRef::docket_job(
            projection.job.id.to_string(),
            docket_checkout_refs(projection.job.id, parent_task_id),
        ),
        items,
        current_item,
        source_conversation_id: projection.job.source_conversation_id.clone(),
        source_client_session_id: None,
        handoff_intent_path: None,
        handoff_task_id: None,
        created_at: projection.job.created_at,
        updated_at: projection.job.updated_at,
    }
}

pub fn task_list_projection_from_session_tasks(
    bear_id: Uuid,
    owner_profile: BearProfile,
    conversation_id: &str,
    session_anchor_id: Uuid,
    tasks: &[DocketTaskProjection],
) -> Option<TaskListProjection> {
    let first_task = tasks.first()?;
    let mut sorted_tasks = tasks.iter().collect::<Vec<_>>();
    // See task_list_projection_from_docket_job: ACP plan projection must be
    // deterministic even if the caller hands us an unordered task slice.
    sorted_tasks.sort_by_key(|projection| {
        (
            projection.task.sibling_order,
            projection.task.created_at,
            projection.task.id,
        )
    });
    let items = sorted_tasks
        .into_iter()
        .map(|projection| {
            task_list_item_from_docket_task(&projection.task, projection.run_state.as_ref())
        })
        .collect::<Vec<_>>();
    let current_item = current_task_list_item(&items).cloned();
    let status = if items
        .iter()
        .all(|item| item.status == TaskListItemStatus::Completed)
    {
        "completed"
    } else if items
        .iter()
        .any(|item| item.status == TaskListItemStatus::InProgress)
    {
        "active"
    } else {
        "planned"
    };
    Some(TaskListProjection {
        id: session_anchor_id,
        bear_id,
        title: "Session tasks".to_string(),
        summary: "Tasks anchored to the current client session".to_string(),
        owner_profile: owner_profile.as_str().to_string(),
        visibility: "private_to_profile".to_string(),
        status: status.to_string(),
        version: 1,
        source_ref: TaskListSourceRef::local(vec![format!("session_anchor:{session_anchor_id}")]),
        items,
        current_item,
        source_conversation_id: Some(conversation_id.to_string()),
        source_client_session_id: Some(session_anchor_id.to_string()),
        handoff_intent_path: None,
        handoff_task_id: None,
        created_at: first_task.task.created_at,
        updated_at: tasks
            .iter()
            .map(|projection| projection.task.updated_at)
            .max()
            .unwrap_or(first_task.task.updated_at),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocketSourceRef {
    Job(Uuid),
    ParentTask(Uuid),
    Task(Uuid),
    MissingJob,
}

impl DocketSourceRef {
    const JOB_PREFIX: &'static str = "docket_job:";
    const PARENT_TASK_PREFIX: &'static str = "docket_parent_task:";
    const TASK_PREFIX: &'static str = "docket_task:";
    const MISSING_JOB_REF: &'static str = "docket_job:<none>";

    fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw == Self::MISSING_JOB_REF {
            return Some(Self::MissingJob);
        }
        if let Some(id) = raw.strip_prefix(Self::JOB_PREFIX) {
            return Uuid::parse_str(id.trim()).ok().map(Self::Job);
        }
        if let Some(id) = raw.strip_prefix(Self::PARENT_TASK_PREFIX) {
            return Uuid::parse_str(id.trim()).ok().map(Self::ParentTask);
        }
        if let Some(id) = raw.strip_prefix(Self::TASK_PREFIX) {
            return Uuid::parse_str(id.trim()).ok().map(Self::Task);
        }
        None
    }
}

impl fmt::Display for DocketSourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Job(id) => write!(f, "{}{id}", Self::JOB_PREFIX),
            Self::ParentTask(id) => write!(f, "{}{id}", Self::PARENT_TASK_PREFIX),
            Self::Task(id) => write!(f, "{}{id}", Self::TASK_PREFIX),
            Self::MissingJob => f.write_str(Self::MISSING_JOB_REF),
        }
    }
}

fn docket_checkout_refs(job_id: Uuid, parent_task_id: Option<Uuid>) -> Vec<String> {
    let mut refs = vec![DocketSourceRef::Job(job_id).to_string()];
    if let Some(parent_task_id) = parent_task_id {
        refs.push(DocketSourceRef::ParentTask(parent_task_id).to_string());
    }
    refs
}

pub fn docket_parent_task_ref(source_ref: &TaskListSourceRef) -> Option<Uuid> {
    source_ref.refs.iter().find_map(|raw| {
        if let Some(DocketSourceRef::ParentTask(id)) = DocketSourceRef::parse(raw) {
            Some(id)
        } else {
            None
        }
    })
}

pub fn task_list_item_status_from_docket_task_status(status: &str) -> TaskListItemStatus {
    match status {
        "in_progress" => TaskListItemStatus::InProgress,
        "done" => TaskListItemStatus::Completed,
        "blocked" => TaskListItemStatus::Blocked,
        "cancelled" => TaskListItemStatus::Cancelled,
        _ => TaskListItemStatus::Pending,
    }
}

pub fn docket_task_status_from_task_list_item_status(
    status: TaskListItemStatus,
) -> DocketTaskStatus {
    match status {
        TaskListItemStatus::Pending => DocketTaskStatus::Pending,
        TaskListItemStatus::InProgress => DocketTaskStatus::InProgress,
        TaskListItemStatus::Blocked => DocketTaskStatus::Blocked,
        TaskListItemStatus::Completed => DocketTaskStatus::Done,
        TaskListItemStatus::Cancelled => DocketTaskStatus::Cancelled,
    }
}

fn task_list_item_from_docket_task(
    task: &DocketTaskRow,
    state: Option<&DocketTaskRunStateRow>,
) -> TaskListItem {
    let status = state
        .map(|state| task_list_item_status_from_docket_task_status(&state.status))
        .unwrap_or(TaskListItemStatus::Pending);
    TaskListItem {
        id: task.id.to_string(),
        title: task.title.clone(),
        summary: Some(task.body.clone()),
        status,
        blocked_reason: (status == TaskListItemStatus::Blocked)
            .then(|| state.and_then(|state| state.result_summary.clone()))
            .flatten(),
        source_ref: TaskListSourceRef::docket_task(
            task.job_id.map(|job_id| job_id.to_string()),
            task.id.to_string(),
            vec![
                task.job_id
                    .map(DocketSourceRef::Job)
                    .unwrap_or(DocketSourceRef::MissingJob)
                    .to_string(),
                DocketSourceRef::Task(task.id).to_string(),
            ],
        ),
        sync_state: TaskListSyncState::Clean,
    }
}

fn current_task_list_item(items: &[TaskListItem]) -> Option<&TaskListItem> {
    items
        .iter()
        .find(|item| item.status == TaskListItemStatus::InProgress)
        .or_else(|| {
            items
                .iter()
                .find(|item| item.status == TaskListItemStatus::Blocked)
        })
        .or_else(|| {
            items
                .iter()
                .find(|item| item.status == TaskListItemStatus::Pending)
        })
}

pub fn validate_docket_job_create(create: &DocketJobCreate) -> Result<(), DocketValidationError> {
    if create.goal.trim().is_empty() {
        return Err(DocketValidationError::EmptyGoal);
    }
    if create.work_surface_id.is_none() {
        return Err(DocketValidationError::MissingWorkSurface);
    }
    if !matches!(create.created_by_role.trim(), "chat" | "pair" | "ui") {
        return Err(DocketValidationError::InvalidJobCreatorRole {
            role: create.created_by_role.clone(),
        });
    }
    for criterion in &create.criteria {
        if criterion.description.trim().is_empty() {
            return Err(DocketValidationError::EmptyCriterionDescription);
        }
    }
    validate_docket_task_inputs(&create.tasks)
}

pub fn normalize_completion_criteria(criteria: &[String]) -> Vec<String> {
    criteria
        .iter()
        .map(|criterion| criterion.trim())
        .filter(|criterion| !criterion.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn validate_completion_criteria(criteria: &[String]) -> Result<(), DocketValidationError> {
    if criteria.is_empty() {
        return Err(DocketValidationError::EmptyTaskCompletionCriteria);
    }
    if criteria.iter().any(|criterion| criterion.trim().is_empty()) {
        return Err(DocketValidationError::EmptyTaskCompletionCriterion);
    }
    Ok(())
}

pub fn validate_docket_task_create(create: &DocketTaskCreate) -> Result<(), DocketValidationError> {
    if create.job_id.is_none() && create.session_anchor_id.is_none() {
        return Err(DocketValidationError::TaskMissingAnchor);
    }
    if create.title.trim().is_empty() {
        return Err(DocketValidationError::EmptyTaskTitle);
    }
    if create.body.trim().is_empty() {
        return Err(DocketValidationError::EmptyTaskBody);
    }
    validate_completion_criteria(&create.completion_criteria)?;
    Ok(())
}

fn validate_docket_task_inputs(tasks: &[DocketTaskInput]) -> Result<(), DocketValidationError> {
    let mut keys = std::collections::HashSet::new();
    for task in tasks {
        if task.title.trim().is_empty() {
            return Err(DocketValidationError::EmptyTaskTitle);
        }
        if task.body.trim().is_empty() {
            return Err(DocketValidationError::EmptyTaskBody);
        }
        validate_completion_criteria(&task.completion_criteria)?;
        if let Some(key) = task
            .client_key
            .as_ref()
            .map(|key| key.trim())
            .filter(|key| !key.is_empty())
        {
            if !keys.insert(key.to_string()) {
                return Err(DocketValidationError::DuplicateTaskClientKey {
                    client_key: key.to_string(),
                });
            }
        }
    }
    for task in tasks {
        if let Some(parent_key) = task
            .parent_client_key
            .as_ref()
            .map(|key| key.trim())
            .filter(|key| !key.is_empty())
        {
            if !keys.contains(parent_key) {
                return Err(DocketValidationError::MissingParentClientKey {
                    client_key: parent_key.to_string(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn current_item(items: &[TaskListUpdateItem]) -> Option<&TaskListUpdateItem> {
    items
        .iter()
        .find(|item| item.status == TaskListItemStatus::InProgress)
        .or_else(|| {
            items
                .iter()
                .find(|item| item.status == TaskListItemStatus::Blocked)
        })
        .or_else(|| {
            items
                .iter()
                .find(|item| item.status == TaskListItemStatus::Pending)
        })
}

pub fn validate_task_list_update(update: &TaskListUpdate) -> Result<(), TaskListValidationError> {
    if update.title.trim().is_empty() {
        return Err(TaskListValidationError::EmptyTitle);
    }

    validate_task_list_items(&update.items)
}

pub fn validate_task_list_items(
    items: &[TaskListUpdateItem],
) -> Result<(), TaskListValidationError> {
    let mut in_progress_count = 0;
    for item in items {
        if item.id.trim().is_empty() {
            return Err(TaskListValidationError::EmptyItemId);
        }
        if item.title.trim().is_empty() {
            return Err(TaskListValidationError::EmptyItemTitle {
                item_id: item.id.clone(),
            });
        }
        if item.status == TaskListItemStatus::InProgress {
            in_progress_count += 1;
        }
        if item.status == TaskListItemStatus::Blocked
            && item
                .blocked_reason
                .as_deref()
                .map(|reason| reason.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(TaskListValidationError::BlockedItemMissingReason {
                item_id: item.id.clone(),
            });
        }
    }

    if in_progress_count > 1 {
        return Err(TaskListValidationError::MultipleInProgressItems);
    }
    Ok(())
}

pub fn role_can_update_task_list(role: BearProfile) -> bool {
    matches!(
        role,
        BearProfile::Chat | BearProfile::Pair | BearProfile::Work
    )
}

pub fn role_can_request_task_list_handoff(role: BearProfile) -> bool {
    matches!(role, BearProfile::Chat | BearProfile::Pair)
}

pub fn role_can_read_task_list(
    viewer_role: BearProfile,
    owner_profile: BearProfile,
    visibility: TaskListVisibility,
    same_user: bool,
) -> bool {
    match visibility {
        TaskListVisibility::PrivateToProfile => viewer_role == owner_profile,
        TaskListVisibility::SameUser => same_user || viewer_role == owner_profile,
        TaskListVisibility::BearVisible => true,
        TaskListVisibility::HandoffRequested => {
            matches!(viewer_role, BearProfile::Curate) || viewer_role == owner_profile
        }
    }
}

pub fn render_task_list_prompt_context(task_lists: &[TaskListLocalProjection]) -> String {
    if task_lists.is_empty() {
        return String::new();
    }

    let mut out = String::from(
        "\n\n<system-reminder>\nDen activity context for this Bear. Session task lists are working projections; durable jobs/tasks live in Docket. Use Docket job/task tools for canonical state and `den.task_list.request_handoff` for reviewed promotion or reconciliation.\n",
    );
    for task_list in task_lists.iter().take(5) {
        let _ = write!(
            out,
            "- task_list_id={} owner={} status={} visibility={} title={}",
            task_list.id,
            task_list.owner_profile,
            task_list.status,
            task_list.visibility,
            task_list.title
        );
        if let Some(current) = task_list.current_item.as_ref() {
            let _ = write!(out, " current_item={} ({})", current.title, current.status);
        }
        if !task_list.summary.trim().is_empty() {
            let _ = write!(out, " summary={}", task_list.summary.trim());
        }
        out.push('\n');
    }
    out.push_str("</system-reminder>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, status: TaskListItemStatus) -> TaskListUpdateItem {
        TaskListUpdateItem {
            id: id.to_string(),
            title: format!("Item {id}"),
            summary: None,
            status,
            blocked_reason: None,
            source_refs: Vec::new(),
        }
    }

    #[test]
    fn validates_single_in_progress_item() {
        let items = vec![
            item("one", TaskListItemStatus::Completed),
            item("two", TaskListItemStatus::InProgress),
            item("three", TaskListItemStatus::Pending),
        ];
        assert!(validate_task_list_items(&items).is_ok());
    }

    #[test]
    fn docket_parent_task_ref_uses_typed_source_ref_parser() {
        let parent_id = Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap();
        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap();
        let source_ref = TaskListSourceRef::docket_job(
            task_id.to_string(),
            vec![
                DocketSourceRef::Task(task_id).to_string(),
                format!("  {}  ", DocketSourceRef::ParentTask(parent_id)),
            ],
        );

        assert_eq!(docket_parent_task_ref(&source_ref), Some(parent_id));
        assert_eq!(
            DocketSourceRef::parse(&DocketSourceRef::ParentTask(parent_id).to_string()),
            Some(DocketSourceRef::ParentTask(parent_id))
        );
    }

    #[test]
    fn deserializes_missing_item_ids_with_stable_generated_slugs() {
        let update: TaskListUpdate = serde_json::from_value(serde_json::json!({
            "title": "Fix ACP plan visibility",
            "visibility": "private_to_profile",
            "status": "active",
            "items": [
                { "title": "Inspect BearWire logs", "status": "completed" },
                { "title": "Patch plan projection", "summary": "Surface plan_update in ACP", "status": "in_progress" }
            ]
        }))
        .expect("task-list update should deserialize with generated item ids");
        let repeated: TaskListUpdate = serde_json::from_value(serde_json::json!({
            "title": "Fix ACP plan visibility",
            "visibility": "private_to_profile",
            "status": "active",
            "items": [
                { "title": "Inspect BearWire logs", "status": "completed" },
                { "title": "Patch plan projection", "summary": "Surface plan_update in ACP", "status": "in_progress" }
            ]
        }))
        .expect("task-list update should deserialize repeatedly");

        assert_eq!(update.items.len(), 2);
        assert!(update.items[0].id.starts_with("inspect_bearwire_logs_"));
        assert!(update.items[1].id.starts_with("patch_plan_projection_"));
        assert_eq!(update.items[0].id, repeated.items[0].id);
        assert_eq!(update.items[1].id, repeated.items[1].id);
        assert!(validate_task_list_update(&update).is_ok());
    }

    #[test]
    fn generated_item_ids_are_unique_for_duplicate_items() {
        let update: TaskListUpdate = serde_json::from_value(serde_json::json!({
            "title": "Duplicate item test",
            "visibility": "private_to_profile",
            "status": "active",
            "items": [
                { "title": "Do the thing", "status": "pending" },
                { "title": "Do the thing", "status": "pending" }
            ]
        }))
        .expect("task-list update should deserialize duplicate generated ids");

        assert_ne!(update.items[0].id, update.items[1].id);
        assert!(update.items[0].id.starts_with("do_the_thing_"));
        assert!(update.items[1].id.starts_with("do_the_thing_"));
        assert!(validate_task_list_update(&update).is_ok());
    }

    #[test]
    fn rejects_multiple_in_progress_items() {
        let items = vec![
            item("one", TaskListItemStatus::InProgress),
            item("two", TaskListItemStatus::InProgress),
        ];
        assert_eq!(
            validate_task_list_items(&items),
            Err(TaskListValidationError::MultipleInProgressItems)
        );
    }

    #[test]
    fn blocked_items_need_reason() {
        let items = vec![item("one", TaskListItemStatus::Blocked)];
        assert_eq!(
            validate_task_list_items(&items),
            Err(TaskListValidationError::BlockedItemMissingReason {
                item_id: "one".to_string()
            })
        );
    }

    #[test]
    fn visibility_preserves_role_boundaries() {
        assert!(role_can_read_task_list(
            BearProfile::Pair,
            BearProfile::Pair,
            TaskListVisibility::PrivateToProfile,
            false
        ));
        assert!(!role_can_read_task_list(
            BearProfile::Chat,
            BearProfile::Pair,
            TaskListVisibility::PrivateToProfile,
            false
        ));
        assert!(role_can_read_task_list(
            BearProfile::Chat,
            BearProfile::Pair,
            TaskListVisibility::BearVisible,
            false
        ));
        assert!(role_can_read_task_list(
            BearProfile::Curate,
            BearProfile::Pair,
            TaskListVisibility::HandoffRequested,
            false
        ));
        assert!(!role_can_read_task_list(
            BearProfile::Work,
            BearProfile::Pair,
            TaskListVisibility::HandoffRequested,
            false
        ));
    }

    #[test]
    fn only_channel_roles_request_task_list_handoff() {
        assert!(role_can_request_task_list_handoff(BearProfile::Chat));
        assert!(role_can_request_task_list_handoff(BearProfile::Pair));
        assert!(!role_can_request_task_list_handoff(BearProfile::Work));
        assert!(!role_can_request_task_list_handoff(BearProfile::Curate));
    }

    fn projection_fixture(items: Vec<TaskListUpdateItem>) -> TaskListLocalProjection {
        TaskListLocalProjection {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap(),
            bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
            title: "Build task system".to_string(),
            summary: "Keep status current".to_string(),
            owner_profile: "pair".to_string(),
            visibility: "bear_visible".to_string(),
            status: "active".to_string(),
            version: 1,
            current_item: current_item(&items).cloned(),
            items,
            source_conversation_id: Some("den-conv-test".to_string()),
            source_client_session_id: Some("acp-test".to_string()),
            handoff_intent_path: None,
            handoff_task_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn task_list_projection_wraps_task_list_as_local_projection() {
        let plan = projection_fixture(vec![item("one", TaskListItemStatus::InProgress)]);

        let task_list = plan.to_task_list_projection();

        assert_eq!(task_list.source_ref.kind, "local");
        assert_eq!(
            task_list.source_ref.refs,
            vec![format!("task_list:{}", plan.id)]
        );
        assert_eq!(task_list.items.len(), 1);
        assert_eq!(task_list.items[0].source_ref.kind, "local");
        assert_eq!(task_list.items[0].sync_state, TaskListSyncState::LocalOnly);
        assert_eq!(
            task_list.current_item.as_ref().map(|item| item.id.as_str()),
            Some("one")
        );
    }

    #[test]
    fn task_list_projection_preserves_docket_backing_refs_when_present() {
        let mut backed = item("backed", TaskListItemStatus::Pending);
        backed.source_refs = vec![
            "docket_job:job-123".to_string(),
            "docket_task:task-456".to_string(),
        ];
        let plan = projection_fixture(vec![backed]);

        let task_list = task_list_projection_from_local(&plan);
        let item = &task_list.items[0];

        assert_eq!(item.source_ref.kind, "docket_task");
        assert_eq!(item.source_ref.docket_job_id.as_deref(), Some("job-123"));
        assert_eq!(item.source_ref.docket_task_id.as_deref(), Some("task-456"));
        assert_eq!(item.sync_state, TaskListSyncState::CheckedOut);
    }

    #[test]
    fn session_task_projection_is_planned_until_work_starts() {
        let bear_id = Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap();
        let session_anchor_id = Uuid::parse_str("00000000-0000-0000-0000-000000000789").unwrap();
        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let run_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let task = DocketTaskRow {
            id: task_id,
            bear_id,
            job_id: None,
            session_anchor_id: Some(session_anchor_id),
            parent_task_id: None,
            sibling_order: 0,
            kind: "execution".to_string(),
            scope: "run".to_string(),
            title: "Check the plan".to_string(),
            body: "Do not start yet.".to_string(),
            completion_criteria: Json(vec!["Plan is visible".to_string()]),
            difficulty: None,
            effort_hint: None,
            routing_strategy: "auto".to_string(),
            expected_context_size: None,
            result_rollup_policy: None,
            created_by_role: "pair".to_string(),
            created_by_user_id: None,
            created_by_agent_id: None,
            created_in_run_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };
        let planned = task_list_projection_from_session_tasks(
            bear_id,
            BearProfile::Pair,
            "conversation-1",
            session_anchor_id,
            &[DocketTaskProjection {
                task: task.clone(),
                run_state: None,
            }],
        )
        .expect("session projection");

        assert_eq!(planned.status, "planned");
        assert_eq!(
            planned.current_item.as_ref().map(|item| item.id.as_str()),
            Some(task_id.to_string().as_str())
        );

        let active = task_list_projection_from_session_tasks(
            bear_id,
            BearProfile::Pair,
            "conversation-1",
            session_anchor_id,
            &[DocketTaskProjection {
                task: task.clone(),
                run_state: Some(DocketTaskRunStateRow {
                    run_id,
                    task_id,
                    status: "in_progress".to_string(),
                    result_refs: None,
                    result_summary: None,
                    started_at: None,
                    finished_at: None,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                }),
            }],
        )
        .expect("active session projection");

        assert_eq!(active.status, "active");

        let completed = task_list_projection_from_session_tasks(
            bear_id,
            BearProfile::Pair,
            "conversation-1",
            session_anchor_id,
            &[DocketTaskProjection {
                task,
                run_state: Some(DocketTaskRunStateRow {
                    run_id,
                    task_id,
                    status: "done".to_string(),
                    result_refs: None,
                    result_summary: None,
                    started_at: None,
                    finished_at: Some(OffsetDateTime::UNIX_EPOCH),
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                }),
            }],
        )
        .expect("completed session projection");

        assert_eq!(completed.status, "completed");
    }

    #[test]
    fn docket_job_status_serializes_archived() {
        assert_eq!(DocketJobStatus::Archived.as_str(), "archived");
        assert_eq!(
            serde_json::to_string(&DocketJobStatus::Archived).unwrap(),
            "\"archived\""
        );
        assert_eq!(
            serde_json::from_str::<DocketJobStatus>("\"archived\"").unwrap(),
            DocketJobStatus::Archived
        );
    }

    #[test]
    fn validates_docket_job_created_by_human_surface() {
        let create = DocketJobCreate {
            bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
            created_by_user_id: 42,
            created_by_role: "work".to_string(),
            goal: "Ship Docket".to_string(),
            work_surface_id: Some(Uuid::parse_str("00000000-0000-0000-0000-000000000999").unwrap()),
            commit_policy: Some(DocketCommitPolicy::None),
            work_branch: None,
            status: DocketJobStatus::Ready,
            visibility: TaskListVisibility::BearVisible,
            source_conversation_id: None,
            objective_kind: None,
            supersedes_job_id: None,
            overlap_resolution: DocketJobOverlapResolution::Reject,
            criteria: Vec::new(),
            tasks: Vec::new(),
        };

        assert_eq!(
            validate_docket_job_create(&create),
            Err(DocketValidationError::InvalidJobCreatorRole {
                role: "work".to_string()
            })
        );
    }

    #[test]
    fn validates_docket_task_client_key_hierarchy() {
        let create = DocketJobCreate {
            bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
            created_by_user_id: 42,
            created_by_role: "pair".to_string(),
            goal: "Ship Docket".to_string(),
            work_surface_id: Some(Uuid::parse_str("00000000-0000-0000-0000-000000000999").unwrap()),
            commit_policy: None,
            work_branch: None,
            status: DocketJobStatus::Ready,
            visibility: TaskListVisibility::BearVisible,
            source_conversation_id: None,
            objective_kind: None,
            supersedes_job_id: None,
            overlap_resolution: DocketJobOverlapResolution::Reject,
            criteria: Vec::new(),
            tasks: vec![DocketTaskInput {
                client_key: Some("child".to_string()),
                parent_client_key: Some("missing-parent".to_string()),
                parent_task_id: None,
                sibling_order: 0,
                kind: DocketTaskKind::Execution,
                scope: DocketTaskScope::Template,
                title: "Implement child".to_string(),
                body: "Do the child task.".to_string(),
                completion_criteria: vec!["Child task is actually done".to_string()],
                difficulty: Some(DocketTaskDifficulty::Moderate),
                effort_hint: Some(DocketEffortHint::Medium),
                routing_strategy: RoutingStrategy::Auto,
                expected_context_size: None,
                result_rollup_policy: None,
            }],
        };

        assert_eq!(
            validate_docket_job_create(&create),
            Err(DocketValidationError::MissingParentClientKey {
                client_key: "missing-parent".to_string()
            })
        );
    }

    #[test]
    fn rejects_docket_task_without_completion_criteria() {
        let create = DocketTaskCreate {
            bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
            job_id: Some(Uuid::parse_str("00000000-0000-0000-0000-000000000777").unwrap()),
            session_anchor_id: None,
            parent_task_id: None,
            sibling_order: 0,
            kind: DocketTaskKind::Investigation,
            scope: DocketTaskScope::Run,
            title: "Investigate".to_string(),
            body: "Find the relevant facts.".to_string(),
            completion_criteria: Vec::new(),
            difficulty: Some(DocketTaskDifficulty::Unknown),
            effort_hint: None,
            routing_strategy: RoutingStrategy::Auto,
            expected_context_size: None,
            result_rollup_policy: None,
            created_by_role: "pair".to_string(),
            created_by_user_id: Some(42),
            created_by_agent_id: None,
            created_in_run_id: None,
        };

        assert_eq!(
            validate_docket_task_create(&create),
            Err(DocketValidationError::EmptyTaskCompletionCriteria)
        );
    }

    #[test]
    fn validates_session_anchored_or_job_anchored_task_create() {
        let create = DocketTaskCreate {
            bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
            job_id: None,
            session_anchor_id: None,
            parent_task_id: None,
            sibling_order: 0,
            kind: DocketTaskKind::Investigation,
            scope: DocketTaskScope::Run,
            title: "Investigate".to_string(),
            body: "Find the relevant facts.".to_string(),
            completion_criteria: vec!["Relevant facts are identified".to_string()],
            difficulty: Some(DocketTaskDifficulty::Unknown),
            effort_hint: None,
            routing_strategy: RoutingStrategy::Auto,
            expected_context_size: None,
            result_rollup_policy: None,
            created_by_role: "pair".to_string(),
            created_by_user_id: Some(42),
            created_by_agent_id: None,
            created_in_run_id: None,
        };

        assert_eq!(
            validate_docket_task_create(&create),
            Err(DocketValidationError::TaskMissingAnchor)
        );
    }

    fn docket_projection_fixture() -> DocketJobProjection {
        let job_id = Uuid::parse_str("00000000-0000-0000-0000-000000000777").unwrap();
        let root_task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000888").unwrap();
        let run_id = Uuid::parse_str("00000000-0000-0000-0000-000000000abc").unwrap();
        DocketJobProjection {
            job: DocketJobRow {
                id: job_id,
                bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
                created_by_user_id: 42,
                created_by_role: "pair".to_string(),
                goal: "Ship Docket".to_string(),
                work_surface_id: None,
                commit_policy: Some("none".to_string()),
                work_branch: None,
                status: "running".to_string(),
                visibility: "bear_visible".to_string(),
                source_conversation_id: Some("conversation-1".to_string()),
                objective_kind: Some("conversation_task_list".to_string()),
                supersedes_job_id: None,
                current_run_id: Some(run_id),
                created_at: OffsetDateTime::UNIX_EPOCH,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            },
            current_run: Some(DocketJobRunRow {
                id: run_id,
                job_id,
                trigger: "manual".to_string(),
                schedule_ref: None,
                state: "running".to_string(),
                started_at: Some(OffsetDateTime::UNIX_EPOCH),
                finished_at: None,
                outcome: None,
                created_at: OffsetDateTime::UNIX_EPOCH,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            }),
            criteria: vec![DocketJobCriterionRow {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000c01").unwrap(),
                job_id,
                kind: "narrative".to_string(),
                description: "Criterion".to_string(),
                spec: None,
                sibling_order: 0,
                created_at: OffsetDateTime::UNIX_EPOCH,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            }],
            criteria_states: Vec::new(),
            tasks: vec![DocketTaskRow {
                id: root_task_id,
                bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
                job_id: Some(job_id),
                session_anchor_id: None,
                parent_task_id: None,
                sibling_order: 0,
                kind: "execution".to_string(),
                scope: "template".to_string(),
                title: "Root task".to_string(),
                body: "Do root work.".to_string(),
                completion_criteria: Json(vec!["Root work done".to_string()]),
                difficulty: None,
                effort_hint: None,
                routing_strategy: "auto".to_string(),
                expected_context_size: None,
                result_rollup_policy: None,
                created_by_role: "pair".to_string(),
                created_by_user_id: Some(42),
                created_by_agent_id: None,
                created_in_run_id: None,
                created_at: OffsetDateTime::UNIX_EPOCH,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            }],
            task_states: vec![DocketTaskRunStateRow {
                run_id,
                task_id: root_task_id,
                status: "in_progress".to_string(),
                result_refs: None,
                result_summary: None,
                started_at: None,
                finished_at: None,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            }],
        }
    }

    #[test]
    fn docket_status_report_summarizes_current_task_and_next_action() {
        let projection = docket_projection_fixture();

        let report = docket_job_status_report(&projection);

        assert_eq!(report.job_status, "running");
        assert_eq!(report.run_state.as_deref(), Some("running"));
        assert_eq!(report.current_task_title.as_deref(), Some("Root task"));
        assert_eq!(report.task_counts.in_progress, 1);
        assert_eq!(report.criteria_counts.unmet, 1);
        assert!(!report.tasks_complete);
        assert!(!report.criteria_complete);
        assert_eq!(report.next_action, "continue_current_task");
    }

    #[test]
    fn task_list_projection_from_docket_job_projects_conversation_objective_level() {
        let job_id = Uuid::parse_str("00000000-0000-0000-0000-000000000777").unwrap();
        let root_task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000888").unwrap();
        let root_peer_task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000889").unwrap();
        let child_task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000999").unwrap();
        let run_id = Uuid::parse_str("00000000-0000-0000-0000-000000000abc").unwrap();
        let projection = DocketJobProjection {
            job: DocketJobRow {
                id: job_id,
                bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
                created_by_user_id: 42,
                created_by_role: "pair".to_string(),
                goal: "Ship Docket".to_string(),
                work_surface_id: None,
                commit_policy: Some("none".to_string()),
                work_branch: None,
                status: "running".to_string(),
                visibility: "bear_visible".to_string(),
                source_conversation_id: Some("conversation-1".to_string()),
                objective_kind: Some("conversation_task_list".to_string()),
                supersedes_job_id: None,
                current_run_id: Some(run_id),
                created_at: OffsetDateTime::UNIX_EPOCH,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            },
            current_run: None,
            criteria: Vec::new(),
            tasks: vec![
                DocketTaskRow {
                    id: root_task_id,
                    bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
                    job_id: Some(job_id),
                    session_anchor_id: None,
                    parent_task_id: None,
                    sibling_order: 0,
                    kind: "execution".to_string(),
                    scope: "template".to_string(),
                    title: "Root task".to_string(),
                    body: "Do root work.".to_string(),
                    completion_criteria: sqlx::types::Json(vec!["Root work done".to_string()]),
                    difficulty: None,
                    effort_hint: None,
                    routing_strategy: "auto".to_string(),
                    expected_context_size: None,
                    result_rollup_policy: None,
                    created_by_role: "pair".to_string(),
                    created_by_user_id: Some(42),
                    created_by_agent_id: None,
                    created_in_run_id: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                },
                DocketTaskRow {
                    id: root_peer_task_id,
                    bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
                    job_id: Some(job_id),
                    session_anchor_id: None,
                    parent_task_id: None,
                    sibling_order: -1,
                    kind: "execution".to_string(),
                    scope: "template".to_string(),
                    title: "Root peer task".to_string(),
                    body: "Do peer work.".to_string(),
                    completion_criteria: sqlx::types::Json(vec!["Peer work done".to_string()]),
                    difficulty: None,
                    effort_hint: None,
                    routing_strategy: "auto".to_string(),
                    expected_context_size: None,
                    result_rollup_policy: None,
                    created_by_role: "pair".to_string(),
                    created_by_user_id: Some(42),
                    created_by_agent_id: None,
                    created_in_run_id: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                },
                DocketTaskRow {
                    id: child_task_id,
                    bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
                    job_id: Some(job_id),
                    session_anchor_id: None,
                    parent_task_id: Some(root_task_id),
                    sibling_order: 0,
                    kind: "execution".to_string(),
                    scope: "template".to_string(),
                    title: "Child task".to_string(),
                    body: "Do child work.".to_string(),
                    completion_criteria: sqlx::types::Json(vec!["Child work done".to_string()]),
                    difficulty: None,
                    effort_hint: None,
                    routing_strategy: "auto".to_string(),
                    expected_context_size: None,
                    result_rollup_policy: None,
                    created_by_role: "pair".to_string(),
                    created_by_user_id: Some(42),
                    created_by_agent_id: None,
                    created_in_run_id: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                },
            ],
            criteria_states: Vec::new(),
            task_states: vec![DocketTaskRunStateRow {
                run_id,
                task_id: root_task_id,
                status: "in_progress".to_string(),
                result_refs: None,
                result_summary: None,
                started_at: None,
                finished_at: None,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            }],
        };

        let root_checkout = task_list_projection_from_docket_job(&projection, None);
        assert_eq!(root_checkout.source_ref.kind, "docket_job");
        assert_eq!(
            root_checkout.source_conversation_id.as_deref(),
            Some("conversation-1")
        );
        assert_eq!(
            root_checkout
                .current_item
                .as_ref()
                .map(|item| item.id.as_str()),
            Some(root_task_id.to_string().as_str())
        );
        assert_eq!(root_checkout.items.len(), 2);
        assert_eq!(root_checkout.items[0].id, root_peer_task_id.to_string());
        assert_eq!(root_checkout.items[1].id, root_task_id.to_string());
        assert_eq!(root_checkout.items[1].sync_state, TaskListSyncState::Clean);
        assert_eq!(
            root_checkout.items[1].status,
            TaskListItemStatus::InProgress
        );

        let child_checkout = task_list_projection_from_docket_job(&projection, Some(root_task_id));
        assert_eq!(
            child_checkout.source_conversation_id.as_deref(),
            Some("conversation-1")
        );
        assert_eq!(
            child_checkout
                .current_item
                .as_ref()
                .map(|item| item.id.as_str()),
            Some(child_task_id.to_string().as_str())
        );
        assert_eq!(child_checkout.items.len(), 1);
        assert_eq!(child_checkout.items[0].id, child_task_id.to_string());
        assert_eq!(
            child_checkout.items[0].source_ref.docket_task_id.as_deref(),
            Some(child_task_id.to_string().as_str())
        );
    }

    #[test]
    fn renders_compact_prompt_context_without_raw_workspace_context() {
        let plan = TaskListLocalProjection {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap(),
            bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
            title: "Build task system".to_string(),
            summary: "Keep status current".to_string(),
            owner_profile: "pair".to_string(),
            visibility: "bear_visible".to_string(),
            status: "active".to_string(),
            version: 1,
            items: vec![item("one", TaskListItemStatus::InProgress)],
            current_item: Some(item("one", TaskListItemStatus::InProgress)),
            source_conversation_id: None,
            source_client_session_id: None,
            handoff_intent_path: None,
            handoff_task_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };

        let rendered = render_task_list_prompt_context(&[plan]);
        assert!(rendered.contains("Den activity context"));
        assert!(rendered.contains("task_list_id="));
        assert!(rendered.contains("durable jobs/tasks live in Docket"));
        assert!(rendered.contains("Build task system"));
        assert!(rendered.contains("Item one"));
        assert!(!rendered.contains("workspace_context"));
    }
}
