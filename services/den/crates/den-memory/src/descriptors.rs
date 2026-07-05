//! Descriptor-owned vocabularies for the Bear entity layer (ADR-0042 §4, §11).
//!
//! Entity types, relation types, and handle types are all *descriptor-owned*: adding
//! one is a descriptor edit here, never a scattered `match` arm elsewhere (per `AGENTS.md`).
//! The two hard invariants the rest of the crate leans on:
//!
//! - a relation's `class` (`descriptive | access_bearing`) is fixed by its descriptor and
//!   selects the physical table — writers never choose (ADR-0042 §7);
//! - a handle's `strength` (`strong | weak`) is fixed by its descriptor and drives
//!   resolution (ADR-0042 §11).

use serde::{Deserialize, Serialize};

/// Relation class — a closed, immutable 2-value enum owned by the relation descriptor.
/// It routes writes to the correct physical table, so misfiling is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationClass {
    /// Filter/boost only; never gates visibility.
    Descriptive,
    /// Gates visibility/recall; enforced. Stored in `memory_access_rules` (the audit surface).
    AccessBearing,
}

impl RelationClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Descriptive => "descriptive",
            Self::AccessBearing => "access_bearing",
        }
    }
}

/// How a descriptive relation influences recall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallEffect {
    Boost,
    Gate,
    None,
}

/// Handle strength. Strong handles auto-resolve; weak handles keep an entity provisional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandleStrength {
    Strong,
    Weak,
}

/// Owning registry for an entity type's `canonical_ref`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwningRegistry {
    Cabinet,
    Den,
    Calendar,
    BearLocal,
    External,
}

/// Default trust an entity of a given type starts with ("who asserted the identity").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityTrust {
    Inferred,
    Asserted,
}

impl EntityTrust {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inferred => "inferred",
            Self::Asserted => "asserted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "inferred" => Some(Self::Inferred),
            "asserted" => Some(Self::Asserted),
            _ => None,
        }
    }
}

/// Resolution lifecycle state, shared across all entity types (ADR-0042 §6).
///
/// `observed → provisional → resolved → confirmed`, plus terminal `rejected` and
/// `merged`/`superseded`. Ordered so the access-bearing trust gate can ask
/// "is this entity at least `resolved`?" structurally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionState {
    Rejected,
    Merged,
    Observed,
    Provisional,
    Resolved,
    Confirmed,
}

impl ResolutionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::Merged => "merged",
            Self::Observed => "observed",
            Self::Provisional => "provisional",
            Self::Resolved => "resolved",
            Self::Confirmed => "confirmed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "rejected" => Some(Self::Rejected),
            "merged" | "superseded" => Some(Self::Merged),
            "observed" => Some(Self::Observed),
            "provisional" => Some(Self::Provisional),
            "resolved" => Some(Self::Resolved),
            "confirmed" => Some(Self::Confirmed),
            _ => None,
        }
    }

    /// Access-bearing relations may only target entities at this level or above (ADR-0042 §11).
    pub fn can_bear_access(self) -> bool {
        self >= Self::Resolved
    }

    /// Whether the entity is "live" (a merged/rejected entity is dead and reads forward-point away).
    pub fn is_live(self) -> bool {
        !matches!(self, Self::Merged | Self::Rejected)
    }
}

/// Descriptor for an entity type (v1 set; extend by adding a row).
#[derive(Debug, Clone, Copy)]
pub struct EntityTypeDescriptor {
    pub id: &'static str,
    pub owning_registry: OwningRegistry,
    pub default_trust: EntityTrust,
    pub anchor_eligible: bool,
    pub valid_handle_types: &'static [&'static str],
}

/// Descriptor for a relation type. `class` is immutable and routes the write.
#[derive(Debug, Clone, Copy)]
pub struct RelationDescriptor {
    /// Canonical, dotted, internal id (e.g. `den.memory.relation.subject`).
    pub id: &'static str,
    /// Concise model-facing alias (e.g. `subject`).
    pub alias: &'static str,
    pub class: RelationClass,
    /// `None` ⇒ many allowed; `Some(n)` ⇒ at most n per source record.
    pub max_per_record: Option<u32>,
    /// Entity type ids this relation may target; `["*"]` ⇒ any.
    pub applies_to_entities: &'static [&'static str],
    pub allowed_qualifiers: &'static [&'static str],
    pub recall_effect: RecallEffect,
    pub anchor_projecting: bool,
}

/// Descriptor for a handle type. `strength` drives resolution.
#[derive(Debug, Clone, Copy)]
pub struct HandleTypeDescriptor {
    pub id: &'static str,
    pub strength: HandleStrength,
}

const ANY_ENTITY: &[&str] = &["*"];

/// v1 entity-type vocabulary (ADR-0042 §11). `contact` is a `person` with address-book
/// provenance, not its own type; `place` is reserved.
pub const ENTITY_TYPES: &[EntityTypeDescriptor] = &[
    EntityTypeDescriptor {
        id: "person",
        owning_registry: OwningRegistry::Cabinet,
        default_trust: EntityTrust::Inferred,
        anchor_eligible: true,
        valid_handle_types: &[
            "den_user",
            "session_human",
            "cabinet_ref",
            "slack_user",
            "email",
            "mention",
            "alias",
        ],
    },
    EntityTypeDescriptor {
        id: "org",
        owning_registry: OwningRegistry::Cabinet,
        default_trust: EntityTrust::Inferred,
        anchor_eligible: true,
        valid_handle_types: &["cabinet_ref", "url", "mention", "alias"],
    },
    EntityTypeDescriptor {
        id: "event",
        owning_registry: OwningRegistry::Calendar,
        default_trust: EntityTrust::Inferred,
        anchor_eligible: false,
        valid_handle_types: &["calendar_event_id", "mention", "alias"],
    },
    EntityTypeDescriptor {
        id: "mission",
        owning_registry: OwningRegistry::Cabinet,
        default_trust: EntityTrust::Asserted,
        anchor_eligible: true,
        valid_handle_types: &["cabinet_ref", "mention", "alias"],
    },
    EntityTypeDescriptor {
        id: "domain",
        owning_registry: OwningRegistry::BearLocal,
        default_trust: EntityTrust::Asserted,
        anchor_eligible: true,
        valid_handle_types: &["mention", "alias"],
    },
    EntityTypeDescriptor {
        id: "work_surface",
        owning_registry: OwningRegistry::Den,
        default_trust: EntityTrust::Asserted,
        anchor_eligible: true,
        valid_handle_types: &[
            "git_remote",
            "connection_ref",
            "checkout",
            "workspace_root",
            "url",
        ],
    },
    EntityTypeDescriptor {
        id: "connection",
        owning_registry: OwningRegistry::Den,
        default_trust: EntityTrust::Asserted,
        anchor_eligible: false,
        valid_handle_types: &["connection_ref"],
    },
    EntityTypeDescriptor {
        id: "artifact",
        owning_registry: OwningRegistry::External,
        default_trust: EntityTrust::Inferred,
        anchor_eligible: false,
        valid_handle_types: &["url", "mention"],
    },
];

/// v1 relation-type vocabulary (ADR-0042 §3, §4).
pub const RELATIONS: &[RelationDescriptor] = &[
    // Descriptive — filter/boost only.
    RelationDescriptor {
        id: "den.memory.relation.subject",
        alias: "subject",
        class: RelationClass::Descriptive,
        max_per_record: None,
        applies_to_entities: ANY_ENTITY,
        allowed_qualifiers: &["is_primary", "confidence", "source_handle"],
        recall_effect: RecallEffect::Boost,
        anchor_projecting: true,
    },
    RelationDescriptor {
        id: "den.memory.relation.source",
        alias: "source",
        class: RelationClass::Descriptive,
        max_per_record: None,
        applies_to_entities: ANY_ENTITY,
        allowed_qualifiers: &["confidence", "source_handle"],
        recall_effect: RecallEffect::Boost,
        anchor_projecting: false,
    },
    RelationDescriptor {
        id: "den.memory.relation.participant",
        alias: "participant",
        class: RelationClass::Descriptive,
        max_per_record: None,
        applies_to_entities: ANY_ENTITY,
        allowed_qualifiers: &["confidence", "source_handle"],
        recall_effect: RecallEffect::Boost,
        anchor_projecting: false,
    },
    RelationDescriptor {
        id: "den.memory.relation.applies_when",
        alias: "applies_when",
        class: RelationClass::Descriptive,
        max_per_record: None,
        applies_to_entities: &["event", "work_surface", "mission", "domain"],
        allowed_qualifiers: &["confidence", "source_handle"],
        recall_effect: RecallEffect::Boost,
        anchor_projecting: false,
    },
    // Access-bearing — gate visibility; enforced.
    RelationDescriptor {
        id: "den.memory.relation.audience",
        alias: "audience",
        class: RelationClass::AccessBearing,
        max_per_record: None,
        applies_to_entities: &["person", "org"],
        allowed_qualifiers: &["source_handle"],
        recall_effect: RecallEffect::Gate,
        anchor_projecting: false,
    },
    RelationDescriptor {
        id: "den.memory.relation.confined_to",
        alias: "confined_to",
        class: RelationClass::AccessBearing,
        max_per_record: None,
        applies_to_entities: &["work_surface", "mission", "org"],
        allowed_qualifiers: &["source_handle"],
        recall_effect: RecallEffect::Gate,
        anchor_projecting: false,
    },
];

/// v1 handle-type vocabulary + strength (ADR-0042 §11).
pub const HANDLE_TYPES: &[HandleTypeDescriptor] = &[
    HandleTypeDescriptor {
        id: "den_user",
        strength: HandleStrength::Strong,
    },
    HandleTypeDescriptor {
        id: "session_human",
        strength: HandleStrength::Strong,
    },
    HandleTypeDescriptor {
        id: "git_remote",
        strength: HandleStrength::Strong,
    },
    HandleTypeDescriptor {
        id: "cabinet_ref",
        strength: HandleStrength::Strong,
    },
    HandleTypeDescriptor {
        id: "connection_ref",
        strength: HandleStrength::Strong,
    },
    HandleTypeDescriptor {
        id: "calendar_event_id",
        strength: HandleStrength::Strong,
    },
    HandleTypeDescriptor {
        id: "slack_user",
        strength: HandleStrength::Strong,
    },
    HandleTypeDescriptor {
        id: "email",
        strength: HandleStrength::Strong,
    },
    HandleTypeDescriptor {
        id: "url",
        strength: HandleStrength::Strong,
    },
    // Path-unique but machine-local ⇒ weak.
    HandleTypeDescriptor {
        id: "checkout",
        strength: HandleStrength::Weak,
    },
    HandleTypeDescriptor {
        id: "workspace_root",
        strength: HandleStrength::Weak,
    },
    HandleTypeDescriptor {
        id: "mention",
        strength: HandleStrength::Weak,
    },
    HandleTypeDescriptor {
        id: "alias",
        strength: HandleStrength::Weak,
    },
];

/// Descriptor registry resolver. All lookups go through here; no scattered match arms.
pub fn entity_type(id: &str) -> Option<&'static EntityTypeDescriptor> {
    ENTITY_TYPES.iter().find(|d| d.id == id)
}

/// Resolve a relation by canonical id or model-facing alias (legacy aliases accepted at
/// routing boundaries only; not advertised).
pub fn relation(id_or_alias: &str) -> Option<&'static RelationDescriptor> {
    RELATIONS
        .iter()
        .find(|d| d.id == id_or_alias || d.alias == id_or_alias)
}

/// Deterministic, immutable relation → class lookup (drives table routing).
pub fn relation_class(id_or_alias: &str) -> Option<RelationClass> {
    relation(id_or_alias).map(|d| d.class)
}

pub fn handle_type(id: &str) -> Option<&'static HandleTypeDescriptor> {
    HANDLE_TYPES.iter().find(|d| d.id == id)
}

/// Handle strength for a handle type; unknown handle types are treated as weak (fail-safe:
/// an unknown handle never auto-resolves).
pub fn handle_strength(handle_type_id: &str) -> HandleStrength {
    handle_type(handle_type_id)
        .map(|d| d.strength)
        .unwrap_or(HandleStrength::Weak)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_class_is_deterministic_by_id_and_alias() {
        assert_eq!(relation_class("subject"), Some(RelationClass::Descriptive));
        assert_eq!(
            relation_class("den.memory.relation.subject"),
            Some(RelationClass::Descriptive)
        );
        assert_eq!(
            relation_class("audience"),
            Some(RelationClass::AccessBearing)
        );
        assert_eq!(
            relation_class("confined_to"),
            Some(RelationClass::AccessBearing)
        );
        assert_eq!(relation_class("nope"), None);
    }

    #[test]
    fn handle_strength_classifies_strong_and_weak() {
        assert_eq!(handle_strength("git_remote"), HandleStrength::Strong);
        assert_eq!(handle_strength("email"), HandleStrength::Strong);
        assert_eq!(handle_strength("checkout"), HandleStrength::Weak);
        assert_eq!(handle_strength("mention"), HandleStrength::Weak);
        // Unknown handle types are weak (never auto-resolve).
        assert_eq!(handle_strength("totally_unknown"), HandleStrength::Weak);
    }

    #[test]
    fn resolution_ordering_drives_access_gate() {
        assert!(ResolutionState::Resolved.can_bear_access());
        assert!(ResolutionState::Confirmed.can_bear_access());
        assert!(!ResolutionState::Provisional.can_bear_access());
        assert!(!ResolutionState::Observed.can_bear_access());
        assert!(!ResolutionState::Merged.can_bear_access());
        assert!(ResolutionState::Confirmed > ResolutionState::Resolved);
        assert!(ResolutionState::Resolved > ResolutionState::Provisional);
    }

    #[test]
    fn resolution_state_round_trips() {
        for s in [
            ResolutionState::Observed,
            ResolutionState::Provisional,
            ResolutionState::Resolved,
            ResolutionState::Confirmed,
            ResolutionState::Rejected,
            ResolutionState::Merged,
        ] {
            assert_eq!(ResolutionState::parse(s.as_str()), Some(s));
        }
        // `superseded` is accepted as an alias for `merged`.
        assert_eq!(
            ResolutionState::parse("superseded"),
            Some(ResolutionState::Merged)
        );
    }

    #[test]
    fn every_entity_type_has_known_handle_types() {
        for et in ENTITY_TYPES {
            for ht in et.valid_handle_types {
                assert!(
                    handle_type(ht).is_some(),
                    "entity type {} lists unknown handle type {ht}",
                    et.id
                );
            }
        }
    }

    #[test]
    fn every_relation_targets_known_entity_types() {
        for rel in RELATIONS {
            for target in rel.applies_to_entities {
                assert!(
                    *target == "*" || entity_type(target).is_some(),
                    "relation {} targets unknown entity type {target}",
                    rel.alias
                );
            }
        }
    }
}
