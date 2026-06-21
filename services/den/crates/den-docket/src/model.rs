//! Docket domain types — **legacy activity-board shape** (pre-ADR-0034).
//!
//! These are the current `bear_work_plans` JSONB activity-board types. They are
//! deliberately **not** renamed to ADR-0034 `Job`/`Task`: that ADR specifies a
//! relational shape where a task is a pure definition (status is run-scoped) and
//! a job has no embedded item array. Minting `Job`/`Task` structs over this
//! JSONB shape would be a mislabeled structure. The names stay honest until the
//! relational realization (see `docs/decisions/adr-0034-jobs-and-tasks-work-management.md`
//! and `docs/roadmap/DOCKET_IMPLEMENTATION_PLAN.md`).

use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::FromRow;
use std::fmt::{self, Write as _};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPlanVisibility {
    PrivateToProfile,
    SameUser,
    BearVisible,
    HandoffRequested,
}

impl WorkPlanVisibility {
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
                "unknown work plan visibility: {other}"
            ))),
        }
    }
}

impl fmt::Display for WorkPlanVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPlanStatus {
    Active,
    Blocked,
    Completed,
    Cancelled,
    Archived,
}

impl WorkPlanStatus {
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
                "unknown work plan status: {other}"
            ))),
        }
    }
}

impl fmt::Display for WorkPlanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPlanItemStatus {
    Pending,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}

impl WorkPlanItemStatus {
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

impl fmt::Display for WorkPlanItemStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkPlanItem {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub status: WorkPlanItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkPlanUpdate {
    pub title: String,
    pub summary: String,
    pub visibility: WorkPlanVisibility,
    pub status: WorkPlanStatus,
    pub items: Vec<WorkPlanItem>,
    pub workspace_context: serde_json::Value,
}

impl<'de> Deserialize<'de> for WorkPlanUpdate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawWorkPlanUpdate {
            title: String,
            #[serde(default)]
            summary: String,
            visibility: WorkPlanVisibility,
            status: WorkPlanStatus,
            #[serde(default)]
            items: Vec<WorkPlanItem>,
            #[serde(default = "default_json_object")]
            workspace_context: serde_json::Value,
        }

        let mut raw = RawWorkPlanUpdate::deserialize(deserializer)?;
        normalize_work_plan_item_ids(&mut raw.items);
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

pub fn normalize_work_plan_item_ids(items: &mut [WorkPlanItem]) {
    let mut generated_ids = std::collections::HashSet::new();
    for item in items {
        let trimmed = item.id.trim();
        if trimmed.is_empty() {
            let mut generated = generated_work_plan_item_id(item, None);
            if !generated_ids.insert(generated.clone()) {
                let mut ordinal = 2_u32;
                loop {
                    generated = generated_work_plan_item_id(item, Some(ordinal));
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

fn generated_work_plan_item_id(item: &WorkPlanItem, ordinal: Option<u32>) -> String {
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

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BearWorkPlanRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub title: String,
    pub summary: String,
    pub owner_profile: String,
    pub owner_agent_id: Option<String>,
    pub created_by_user_id: Option<i32>,
    pub source_conversation_id: Option<String>,
    pub source_acp_session_id: Option<String>,
    pub source_channel: Json<serde_json::Value>,
    pub workspace_context: Json<serde_json::Value>,
    pub visibility: String,
    pub status: String,
    pub items: Json<Vec<WorkPlanItem>>,
    pub version: i32,
    pub handoff_intent_path: Option<String>,
    pub handoff_task_id: Option<String>,
    pub archived_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct WorkPlanUpsert {
    pub bear_id: Uuid,
    pub owner_profile: BearProfile,
    pub owner_agent_id: Option<String>,
    pub created_by_user_id: Option<i32>,
    pub source_conversation_id: Option<String>,
    pub source_acp_session_id: Option<String>,
    pub source_channel: serde_json::Value,
    pub plan_id: Option<Uuid>,
    pub expected_version: Option<i32>,
    pub update: WorkPlanUpdate,
}

#[derive(Debug, Clone, Default)]
pub struct WorkPlanListFilter {
    pub statuses: Option<Vec<WorkPlanStatus>>,
    pub owner_profile: Option<BearProfile>,
    pub include_archived: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WorkPlanLookup {
    pub plan_id: Option<Uuid>,
    pub source_conversation_id: Option<String>,
    pub source_acp_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkPlanProjection {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub title: String,
    pub summary: String,
    pub owner_profile: String,
    pub visibility: String,
    pub status: String,
    pub version: i32,
    pub items: Vec<WorkPlanItem>,
    pub current_item: Option<WorkPlanItem>,
    pub source_conversation_id: Option<String>,
    pub source_acp_session_id: Option<String>,
    pub handoff_intent_path: Option<String>,
    pub handoff_task_id: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkPlanValidationError {
    EmptyTitle,
    EmptyItemId,
    EmptyItemTitle { item_id: String },
    MultipleInProgressItems,
    BlockedItemMissingReason { item_id: String },
}

impl fmt::Display for WorkPlanValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTitle => f.write_str("work plan title must not be empty"),
            Self::EmptyItemId => f.write_str("work plan item id must not be empty"),
            Self::EmptyItemTitle { item_id } => {
                write!(f, "work plan item `{item_id}` title must not be empty")
            }
            Self::MultipleInProgressItems => {
                f.write_str("work plan may have at most one in_progress item")
            }
            Self::BlockedItemMissingReason { item_id } => {
                write!(
                    f,
                    "blocked work plan item `{item_id}` must include blocked_reason"
                )
            }
        }
    }
}

impl std::error::Error for WorkPlanValidationError {}

impl From<WorkPlanValidationError> for DenError {
    fn from(err: WorkPlanValidationError) -> Self {
        DenError::ValidationError(err.to_string())
    }
}

impl BearWorkPlanRow {
    pub fn parsed_owner_profile(&self) -> Result<BearProfile, DenError> {
        self.owner_profile.parse().map_err(DenError::Parsing)
    }

    pub fn parsed_visibility(&self) -> Result<WorkPlanVisibility, DenError> {
        WorkPlanVisibility::parse(&self.visibility)
    }

    pub fn parsed_status(&self) -> Result<WorkPlanStatus, DenError> {
        WorkPlanStatus::parse(&self.status)
    }

    pub fn is_visible_to(&self, viewer_role: BearProfile, user_id: i32) -> Result<bool, DenError> {
        let owner_profile = self.parsed_owner_profile()?;
        let visibility = self.parsed_visibility()?;
        let same_user = self.created_by_user_id == Some(user_id);
        Ok(role_can_read_work_plan(
            viewer_role,
            owner_profile,
            visibility,
            same_user,
        ))
    }

    pub fn project_for_profile(
        &self,
        viewer_role: BearProfile,
        user_id: i32,
    ) -> Result<Option<WorkPlanProjection>, DenError> {
        if !self.is_visible_to(viewer_role, user_id)? {
            return Ok(None);
        }
        let items = self.items.0.clone();
        let current_item = current_item(&items).cloned();
        Ok(Some(WorkPlanProjection {
            id: self.id,
            bear_id: self.bear_id,
            title: self.title.clone(),
            summary: self.summary.clone(),
            owner_profile: self.owner_profile.clone(),
            visibility: self.visibility.clone(),
            status: self.status.clone(),
            version: self.version,
            items,
            current_item,
            source_conversation_id: self.source_conversation_id.clone(),
            source_acp_session_id: self.source_acp_session_id.clone(),
            handoff_intent_path: self.handoff_intent_path.clone(),
            handoff_task_id: self.handoff_task_id.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }))
    }
}

fn current_item(items: &[WorkPlanItem]) -> Option<&WorkPlanItem> {
    items
        .iter()
        .find(|item| item.status == WorkPlanItemStatus::InProgress)
        .or_else(|| {
            items
                .iter()
                .find(|item| item.status == WorkPlanItemStatus::Blocked)
        })
        .or_else(|| {
            items
                .iter()
                .find(|item| item.status == WorkPlanItemStatus::Pending)
        })
}

pub fn validate_work_plan_update(update: &WorkPlanUpdate) -> Result<(), WorkPlanValidationError> {
    if update.title.trim().is_empty() {
        return Err(WorkPlanValidationError::EmptyTitle);
    }

    validate_work_plan_items(&update.items)
}

pub fn validate_work_plan_items(items: &[WorkPlanItem]) -> Result<(), WorkPlanValidationError> {
    let mut in_progress_count = 0;
    for item in items {
        if item.id.trim().is_empty() {
            return Err(WorkPlanValidationError::EmptyItemId);
        }
        if item.title.trim().is_empty() {
            return Err(WorkPlanValidationError::EmptyItemTitle {
                item_id: item.id.clone(),
            });
        }
        if item.status == WorkPlanItemStatus::InProgress {
            in_progress_count += 1;
        }
        if item.status == WorkPlanItemStatus::Blocked
            && item
                .blocked_reason
                .as_deref()
                .map(|reason| reason.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(WorkPlanValidationError::BlockedItemMissingReason {
                item_id: item.id.clone(),
            });
        }
    }

    if in_progress_count > 1 {
        return Err(WorkPlanValidationError::MultipleInProgressItems);
    }
    Ok(())
}

pub fn role_can_update_work_plan(role: BearProfile) -> bool {
    matches!(
        role,
        BearProfile::Chat | BearProfile::Pair | BearProfile::Work
    )
}

pub fn role_can_request_work_handoff(role: BearProfile) -> bool {
    matches!(role, BearProfile::Chat | BearProfile::Pair)
}

pub fn role_can_read_work_plan(
    viewer_role: BearProfile,
    owner_profile: BearProfile,
    visibility: WorkPlanVisibility,
    same_user: bool,
) -> bool {
    match visibility {
        WorkPlanVisibility::PrivateToProfile => viewer_role == owner_profile,
        WorkPlanVisibility::SameUser => same_user || viewer_role == owner_profile,
        WorkPlanVisibility::BearVisible => true,
        WorkPlanVisibility::HandoffRequested => {
            matches!(viewer_role, BearProfile::Curate) || viewer_role == owner_profile
        }
    }
}

pub fn render_workboard_prompt_context(plans: &[WorkPlanProjection]) -> String {
    if plans.is_empty() {
        return String::new();
    }

    let mut out = String::from(
        "\n\n<system-reminder>\nDen activity context for this Bear. Use `den.work_plan.update` to keep live activity/status current. Use `den.work_plan.request_handoff` when channel work should become a durable task intent.\n",
    );
    for plan in plans.iter().take(5) {
        let _ = write!(
            out,
            "- plan_id={} owner={} status={} visibility={} title={}",
            plan.id, plan.owner_profile, plan.status, plan.visibility, plan.title
        );
        if let Some(current) = plan.current_item.as_ref() {
            let _ = write!(out, " current_item={} ({})", current.title, current.status);
        }
        if !plan.summary.trim().is_empty() {
            let _ = write!(out, " summary={}", plan.summary.trim());
        }
        out.push('\n');
    }
    out.push_str("</system-reminder>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, status: WorkPlanItemStatus) -> WorkPlanItem {
        WorkPlanItem {
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
            item("one", WorkPlanItemStatus::Completed),
            item("two", WorkPlanItemStatus::InProgress),
            item("three", WorkPlanItemStatus::Pending),
        ];
        assert!(validate_work_plan_items(&items).is_ok());
    }

    #[test]
    fn deserializes_missing_item_ids_with_stable_generated_slugs() {
        let update: WorkPlanUpdate = serde_json::from_value(serde_json::json!({
            "title": "Fix ACP plan visibility",
            "visibility": "private_to_profile",
            "status": "active",
            "items": [
                { "title": "Inspect BearWire logs", "status": "completed" },
                { "title": "Patch plan projection", "summary": "Surface plan_update in ACP", "status": "in_progress" }
            ]
        }))
        .expect("work plan update should deserialize with generated item ids");
        let repeated: WorkPlanUpdate = serde_json::from_value(serde_json::json!({
            "title": "Fix ACP plan visibility",
            "visibility": "private_to_profile",
            "status": "active",
            "items": [
                { "title": "Inspect BearWire logs", "status": "completed" },
                { "title": "Patch plan projection", "summary": "Surface plan_update in ACP", "status": "in_progress" }
            ]
        }))
        .expect("work plan update should deserialize repeatedly");

        assert_eq!(update.items.len(), 2);
        assert!(update.items[0].id.starts_with("inspect_bearwire_logs_"));
        assert!(update.items[1].id.starts_with("patch_plan_projection_"));
        assert_eq!(update.items[0].id, repeated.items[0].id);
        assert_eq!(update.items[1].id, repeated.items[1].id);
        assert!(validate_work_plan_update(&update).is_ok());
    }

    #[test]
    fn generated_item_ids_are_unique_for_duplicate_items() {
        let update: WorkPlanUpdate = serde_json::from_value(serde_json::json!({
            "title": "Duplicate item test",
            "visibility": "private_to_profile",
            "status": "active",
            "items": [
                { "title": "Do the thing", "status": "pending" },
                { "title": "Do the thing", "status": "pending" }
            ]
        }))
        .expect("work plan update should deserialize duplicate generated ids");

        assert_ne!(update.items[0].id, update.items[1].id);
        assert!(update.items[0].id.starts_with("do_the_thing_"));
        assert!(update.items[1].id.starts_with("do_the_thing_"));
        assert!(validate_work_plan_update(&update).is_ok());
    }

    #[test]
    fn rejects_multiple_in_progress_items() {
        let items = vec![
            item("one", WorkPlanItemStatus::InProgress),
            item("two", WorkPlanItemStatus::InProgress),
        ];
        assert_eq!(
            validate_work_plan_items(&items),
            Err(WorkPlanValidationError::MultipleInProgressItems)
        );
    }

    #[test]
    fn blocked_items_need_reason() {
        let items = vec![item("one", WorkPlanItemStatus::Blocked)];
        assert_eq!(
            validate_work_plan_items(&items),
            Err(WorkPlanValidationError::BlockedItemMissingReason {
                item_id: "one".to_string()
            })
        );
    }

    #[test]
    fn visibility_preserves_role_boundaries() {
        assert!(role_can_read_work_plan(
            BearProfile::Pair,
            BearProfile::Pair,
            WorkPlanVisibility::PrivateToProfile,
            false
        ));
        assert!(!role_can_read_work_plan(
            BearProfile::Chat,
            BearProfile::Pair,
            WorkPlanVisibility::PrivateToProfile,
            false
        ));
        assert!(role_can_read_work_plan(
            BearProfile::Chat,
            BearProfile::Pair,
            WorkPlanVisibility::BearVisible,
            false
        ));
        assert!(role_can_read_work_plan(
            BearProfile::Curate,
            BearProfile::Pair,
            WorkPlanVisibility::HandoffRequested,
            false
        ));
        assert!(!role_can_read_work_plan(
            BearProfile::Work,
            BearProfile::Pair,
            WorkPlanVisibility::HandoffRequested,
            false
        ));
    }

    #[test]
    fn only_channel_roles_request_handoff() {
        assert!(role_can_request_work_handoff(BearProfile::Chat));
        assert!(role_can_request_work_handoff(BearProfile::Pair));
        assert!(!role_can_request_work_handoff(BearProfile::Work));
        assert!(!role_can_request_work_handoff(BearProfile::Curate));
    }

    #[test]
    fn renders_compact_prompt_context_without_raw_workspace_context() {
        let plan = WorkPlanProjection {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap(),
            bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
            title: "Build task system".to_string(),
            summary: "Keep status current".to_string(),
            owner_profile: "pair".to_string(),
            visibility: "bear_visible".to_string(),
            status: "active".to_string(),
            version: 1,
            items: vec![item("one", WorkPlanItemStatus::InProgress)],
            current_item: Some(item("one", WorkPlanItemStatus::InProgress)),
            source_conversation_id: None,
            source_acp_session_id: None,
            handoff_intent_path: None,
            handoff_task_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };

        let rendered = render_workboard_prompt_context(&[plan]);
        assert!(rendered.contains("Den activity context"));
        assert!(rendered.contains("den.work_plan.update"));
        assert!(rendered.contains("Build task system"));
        assert!(rendered.contains("Item one"));
        assert!(!rendered.contains("workspace_context"));
    }
}
