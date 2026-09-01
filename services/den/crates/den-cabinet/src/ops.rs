//! Facade operation inputs and result summaries.
//!
//! Every request type carries a required [`ActorScope`]; deserializing a
//! request without one fails. Transport (tool descriptor, HTTP route) is an
//! implementation concern above this crate — names, inputs, outputs, and
//! authority requirements are contract.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::records::{
    AttachmentRole, ItemKind, Lifecycle, ReviewDecision, ReviewState, SourceKind, SourceRole,
};
use crate::refs::{
    CabinetCollectionRef, CabinetItemRef, CabinetSourceRef, CabinetVersionRef, MissionRef,
};
use crate::scope::ActorScope;

/// Filters for `cabinet_search`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_ref: Option<CabinetCollectionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_ref: Option<MissionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<Lifecycle>,
}

/// `cabinet_search(scope, query, filters?)` — requires read authority.
/// Unreadable items are absent from results, not redacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub scope: ActorScope,
    pub query: String,
    #[serde(default)]
    pub filters: SearchFilters,
}

/// `cabinet_read(scope, cabinet_ref, version_ref?)` — requires read authority.
/// Defaults to the item's current version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_ref: Option<CabinetVersionRef>,
}

/// `cabinet_history(scope, cabinet_ref)` — requires read authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
}

/// A source link supplied at item creation or via `cabinet_link_source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSourceLink {
    pub source_kind: SourceKind,
    pub locator: String,
    pub role: SourceRole,
}

/// `cabinet_create_item(...)` — requires write authority. Creates the item and
/// its first published version atomically; scope binding is fixed at creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateItemRequest {
    pub scope: ActorScope,
    pub kind: ItemKind,
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_ref: Option<CabinetCollectionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_ref: Option<MissionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_links: Vec<NewSourceLink>,
}

/// `cabinet_update_item(...)` — requires write authority. Appends a new
/// immutable version and advances `current_version`. A stale `base_version`
/// fails with a structured conflict; the facade never merges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateItemRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
    pub content: String,
    pub base_version: CabinetVersionRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// `cabinet_archive_item(scope, cabinet_ref)` — requires write authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveItemRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
}

/// `cabinet_restore_item(scope, cabinet_ref)` — requires write authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreItemRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
}

/// `cabinet_link_source(...)` — requires write authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSourceRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
    #[serde(flatten)]
    pub link: NewSourceLink,
}

/// `cabinet_unlink_source(...)` — requires write authority. Removes the link
/// record only; versions are never altered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlinkSourceRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
    pub source_ref: CabinetSourceRef,
}

/// `cabinet_organize(...)` — Phase 2, contract-reserved. Rebinds an item's
/// collection/Mission; requires write authority on the item and destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizeRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_ref: Option<CabinetCollectionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_ref: Option<MissionRef>,
}

/// `cabinet_review(...)` — Phase 2, contract-reserved. Requires review
/// authority; approval advances `current_version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
    pub version_ref: CabinetVersionRef,
    pub decision: ReviewDecision,
    pub rationale: String,
}

/// `cabinet_link_attachment(...)` — Phase 3, contract-reserved. Requires write
/// authority plus artifact read authority at link time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkAttachmentRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
    pub artifact_ref: String,
    pub role: AttachmentRole,
}

/// `cabinet_unlink_attachment(...)` — Phase 3, contract-reserved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlinkAttachmentRequest {
    pub scope: ActorScope,
    pub cabinet_ref: CabinetItemRef,
    pub artifact_ref: String,
}

/// Search-result row: identity, current version, and scope binding — enough
/// to decide whether to read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemSummary {
    pub cabinet_ref: CabinetItemRef,
    pub current_version: CabinetVersionRef,
    pub title: String,
    pub kind: ItemKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_ref: Option<CabinetCollectionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_ref: Option<MissionRef>,
    pub lifecycle: Lifecycle,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Revision-history row from `cabinet_history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionSummary {
    pub version_ref: CabinetVersionRef,
    pub revision: u32,
    pub authored_by: ActorScope,
    #[serde(with = "time::serde::rfc3339")]
    pub authored_at: OffsetDateTime,
    pub review: ReviewState,
    pub content_sha256: String,
}
