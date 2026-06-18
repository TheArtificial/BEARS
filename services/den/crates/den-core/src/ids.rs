//! Strongly-typed identifier newtypes for the den workspace.
//!
//! These wrap the raw primitives (`Uuid`, `i32`, `String`) that flow through the
//! foundation seams so signatures stop being "stringy"/"uuid-soup" and the
//! compiler can catch id mix-ups (DEN_CRATE_SPLIT_PLAN.md v0 #4). They are
//! transparent for `serde` and `sqlx`, so adopting them in DTOs and query rows
//! is drop-in as modules move into their crates during v1.
//!
//! Scope note: this seeds the canonical id vocabulary in `den-core` and adopts
//! it at foundation boundaries only; the wider call-site sweep rides along with
//! each module move in v1.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifier for a Bear (`bear_id`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct BearId(pub Uuid);

impl BearId {
    #[must_use]
    pub const fn new(id: Uuid) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl From<Uuid> for BearId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl From<BearId> for Uuid {
    fn from(id: BearId) -> Self {
        id.0
    }
}

impl fmt::Display for BearId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for BearId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s.trim()).map(Self)
    }
}

/// Identifier for an authenticated human user (`user_id`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct UserId(pub i32);

impl UserId {
    #[must_use]
    pub const fn new(id: i32) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

impl From<i32> for UserId {
    fn from(id: i32) -> Self {
        Self(id)
    }
}

impl From<UserId> for i32 {
    fn from(id: UserId) -> Self {
        id.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Opaque ACP/runtime session identifier (`session_id`, `acp_session_id`).
///
/// These are provider-/client-issued strings (e.g. `acp-…`, `zed-acp-…`), not
/// UUIDs, so the inner type is `String`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct SessionId(pub String);

/// Opaque conversation identifier (`conversation_id`).
///
/// Conversation ids include sentinel/pending forms (`default`, `new-…`,
/// `conv-…`), so the inner type is `String`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct ConversationId(pub String);

macro_rules! string_id_impls {
    ($ty:ty) => {
        impl $ty {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $ty {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $ty {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id_impls!(SessionId);
string_id_impls!(ConversationId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bear_id_round_trips_through_uuid_and_string() {
        let raw = Uuid::nil();
        let id = BearId::from(raw);
        assert_eq!(id.as_uuid(), raw);
        assert_eq!(Uuid::from(id), raw);
        assert_eq!(id.to_string(), raw.to_string());
        assert_eq!(BearId::from_str(&raw.to_string()).unwrap(), id);
    }

    #[test]
    fn bear_id_serde_is_transparent() {
        let id = BearId::new(Uuid::nil());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", Uuid::nil()));
        let back: BearId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn user_id_orders_and_converts() {
        assert!(UserId::new(1) < UserId::new(2));
        assert_eq!(i32::from(UserId::from(7)), 7);
        assert_eq!(UserId::new(7).get(), 7);
    }

    #[test]
    fn string_ids_are_transparent() {
        let s = SessionId::new("acp-123");
        assert_eq!(s.as_str(), "acp-123");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"acp-123\"");

        let c = ConversationId::from("default");
        assert_eq!(c.to_string(), "default");
        assert_eq!(c.into_string(), "default");
    }
}
