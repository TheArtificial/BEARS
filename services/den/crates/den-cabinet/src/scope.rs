//! Actor scope and authorization vocabulary.
//!
//! Every facade operation takes an explicit [`ActorScope`]: exactly one of a
//! human user or a Bear acting under a stance, plus optional call provenance.
//! There is no ambient, default, or service-identity actor on the model-facing
//! facade. The same type is recorded verbatim as actor provenance on items,
//! versions, and links.

use den_core::ids::{BearId, ConversationId, UserId};
use den_core::profile::BearStance;
use serde::{Deserialize, Serialize};

/// The acting identity: a human user, or a Bear under an operational stance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "actor_kind", rename_all = "snake_case")]
pub enum Actor {
    User { user_id: UserId },
    Bear { bear_id: BearId, stance: BearStance },
}

/// Explicit per-operation actor scope, preserved verbatim as provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorScope {
    #[serde(flatten)]
    pub actor: Actor,
    /// Conversation the call originated from, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    /// Run the call originated from, when the write comes from a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Docket task the call originated from, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

impl ActorScope {
    /// Scope for a human user with no call provenance.
    #[must_use]
    pub fn user(user_id: UserId) -> Self {
        Self {
            actor: Actor::User { user_id },
            conversation_id: None,
            run_id: None,
            task_id: None,
        }
    }

    /// Scope for a Bear under a stance with no call provenance.
    #[must_use]
    pub fn bear(bear_id: BearId, stance: BearStance) -> Self {
        Self {
            actor: Actor::Bear { bear_id, stance },
            conversation_id: None,
            run_id: None,
            task_id: None,
        }
    }
}

/// The authority a Cabinet operation requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    Read,
    Write,
    Review,
}

/// Why an authorization decision denied access. Denials are logged with their
/// reason; read/search denial is never disclosed to the caller beyond
/// `NotFound`/absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum DenialReason {
    /// The actor is not a member of this Den.
    NotDenMember,
    /// The item or collection is Mission-bound and the actor is not a member.
    NotMissionMember,
    /// Collection, kind, or stance policy disallows the operation.
    PolicyRestriction { detail: String },
}

/// Outcome of an authorization decision. Every mutating decision (allow or
/// deny) is auditable with actor scope, operation, target refs, and reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AuthorizationOutcome {
    Allow,
    Deny { reason: DenialReason },
}
