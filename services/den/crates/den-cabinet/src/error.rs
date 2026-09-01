//! Contract error vocabulary: construction/validation failures and the stable
//! operation error taxonomy from `docs/architecture/cabinet-contract.md`.

use crate::refs::CabinetVersionRef;

/// A record or ref failed the Phase 0 contract rules at construction or
/// deserialization time.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractViolation {
    #[error("malformed {expected} ref: {value:?}")]
    MalformedRef {
        expected: &'static str,
        value: String,
    },
    #[error("{record} is missing required field {field}")]
    MissingField {
        record: &'static str,
        field: &'static str,
    },
    #[error("{record}.{field} must not be empty")]
    EmptyField {
        record: &'static str,
        field: &'static str,
    },
    #[error("item version revision must start at 1 (got {revision})")]
    RevisionOutOfRange { revision: u32 },
    #[error("item version revision {revision} requires a base_version")]
    MissingBaseVersion { revision: u32 },
    #[error("item version revision 1 must not carry a base_version")]
    UnexpectedBaseVersion,
    #[error("content_sha256 {declared} does not match content (expected {computed})")]
    ContentHashMismatch { declared: String, computed: String },
    #[error("review state {state} is not available under Phase 1 direct-edit")]
    ReviewStateNotAvailable { state: &'static str },
    #[error("source locator {locator:?} does not match source kind {kind}")]
    SourceLocatorMismatch { kind: &'static str, locator: String },
}

/// Stable operation error taxonomy for the Cabinet facade.
///
/// `NotFound` is also the response for unauthorized reads: denial must not
/// confirm existence. `NotAuthorized` is reserved for writes against material
/// the actor can already read (and for blanket capability denials that reveal
/// nothing about specific items).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CabinetError {
    #[error("cabinet item not found")]
    NotFound,
    #[error("not authorized for this cabinet operation")]
    NotAuthorized,
    #[error("stale base_version; current version is {current_version}")]
    Conflict { current_version: CabinetVersionRef },
    #[error("cabinet validation error: {0}")]
    Validation(#[from] ContractViolation),
    #[error("cabinet policy error: {0}")]
    Policy(String),
    /// Storage/system failure behind the facade; not part of the contract's
    /// model-facing taxonomy.
    #[error("cabinet storage error: {0}")]
    Storage(String),
}

impl From<CabinetError> for den_core::DenError {
    fn from(error: CabinetError) -> Self {
        match &error {
            CabinetError::NotFound => Self::NotFound(error.to_string()),
            CabinetError::NotAuthorized => Self::Authorization(error.to_string()),
            CabinetError::Conflict { .. }
            | CabinetError::Validation(_)
            | CabinetError::Policy(_) => Self::ValidationError(error.to_string()),
            CabinetError::Storage(cause) => Self::Database(cause.clone()),
        }
    }
}
