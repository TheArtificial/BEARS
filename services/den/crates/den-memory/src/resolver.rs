//! Entity resolution engine (ADR-0042 §6, §11): map incoming identifying signals to a
//! bear-local entity under uncertainty, staying correctable.
//!
//! Rules (type-agnostic):
//! 1. Exact match on a **strong** handle ⇒ resolve to that entity (attach the other handles,
//!    raise resolution/trust per the source).
//! 2. Only **weak** handles ⇒ candidate search; a confident single candidate ⇒ `Candidate`,
//!    otherwise create a new `provisional`/`observed` entity.
//! 3. **Never auto-merge across different strong identities** on similarity.
//! 4. **Conflicting strong matches** (signals point at two different strong-handle entities)
//!    ⇒ `ConflictNeedsReview` — `curate`/human, never automatic.

use den_core::DenError;

use super::descriptors::{self, EntityTrust, HandleStrength, ResolutionState};
use super::entity::{self, EntityRow};
use super::records::BearMemoryStore;

/// One identifying signal observed about an entity.
#[derive(Debug, Clone)]
pub struct Signal {
    pub handle_type: String,
    pub handle_value: String,
    pub source: Option<String>,
}

impl Signal {
    pub fn new(handle_type: impl Into<String>, handle_value: impl Into<String>) -> Self {
        Self {
            handle_type: handle_type.into(),
            handle_value: handle_value.into(),
            source: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    fn strength(&self) -> HandleStrength {
        descriptors::handle_strength(&self.handle_type)
    }
}

/// Outcome of a resolution attempt.
#[derive(Debug, Clone)]
pub enum Resolution {
    /// Resolved (or freshly created from a strong handle) to a single live entity.
    Resolved(EntityRow),
    /// A single confident weak-only candidate; needs confirmation before it gates anything.
    Candidate(EntityRow),
    /// Created a new provisional/observed entity (no confident match).
    Created(EntityRow),
    /// Strong signals point at two different existing entities; route to `curate`/human.
    ConflictNeedsReview { entity_ids: Vec<String> },
}

/// Whether the identity was asserted by an authoritative source (session/Armature token, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assertion {
    /// Authoritative-by-construction (session human, calendar id): enter at `confirmed`/asserted.
    Asserted,
    /// Observed/inferred: strong handles enter at `resolved`/inferred.
    Inferred,
}

/// Resolve a set of signals into an entity of `entity_type`, applying the shared lifecycle.
pub async fn resolve(
    store: &BearMemoryStore,
    entity_type: &str,
    display_name: Option<&str>,
    signals: &[Signal],
    assertion: Assertion,
) -> Result<Resolution, DenError> {
    if descriptors::entity_type(entity_type).is_none() {
        return Err(DenError::ValidationError(format!(
            "unknown entity type: {entity_type}"
        )));
    }

    let (strong, weak): (Vec<&Signal>, Vec<&Signal>) = signals
        .iter()
        .partition(|s| s.strength() == HandleStrength::Strong);

    // Strong-handle path: look for existing live entities carrying each strong handle.
    let mut matched: Vec<EntityRow> = Vec::new();
    for sig in &strong {
        if let Some(found) =
            entity::find_entity_by_handle(store, &sig.handle_type, &sig.handle_value).await?
        {
            if !matched.iter().any(|e| e.entity_id == found.entity_id) {
                matched.push(found);
            }
        }
    }

    if matched.len() > 1 {
        // Conflicting strong identities — never auto-merge.
        return Ok(Resolution::ConflictNeedsReview {
            entity_ids: matched.into_iter().map(|e| e.entity_id).collect(),
        });
    }

    if let Some(target) = matched.into_iter().next() {
        // Exactly one strong match: attach every signal as a handle, raise resolution/trust.
        attach_signals(store, &target.entity_id, signals).await?;
        let (resolution, trust) = entry_levels(assertion, true);
        let raised = std::cmp::max(target.resolution, resolution);
        // Trust only ratchets up: asserted on either side wins.
        let raised_trust = if matches!(target.trust, EntityTrust::Asserted)
            || matches!(trust, EntityTrust::Asserted)
        {
            EntityTrust::Asserted
        } else {
            EntityTrust::Inferred
        };
        entity::set_resolution(store, &target.entity_id, raised, Some(raised_trust)).await?;
        let live = entity::resolve_live_entity(store, &target.entity_id)
            .await?
            .unwrap_or(target);
        return Ok(Resolution::Resolved(live));
    }

    if !strong.is_empty() {
        // Strong signals but no existing match ⇒ a new, confidently-identified entity.
        let (resolution, trust) = entry_levels(assertion, true);
        let created = entity::create_entity(
            store,
            entity_type,
            display_name,
            resolution,
            trust,
            None,
            &serde_json::Value::Object(Default::default()),
        )
        .await?;
        attach_signals(store, &created.entity_id, signals).await?;
        return Ok(Resolution::Resolved(created));
    }

    // Weak-only path: search for a confident candidate among same-type entities.
    if let Some(candidate) = weak_candidate(store, entity_type, &weak, display_name).await? {
        return Ok(Resolution::Candidate(candidate));
    }

    // Nothing confident: create a provisional/observed entity carrying the weak handles.
    let resolution = if weak.is_empty() {
        ResolutionState::Observed
    } else {
        ResolutionState::Provisional
    };
    let created = entity::create_entity(
        store,
        entity_type,
        display_name,
        resolution,
        EntityTrust::Inferred,
        None,
        &serde_json::Value::Object(Default::default()),
    )
    .await?;
    attach_signals(store, &created.entity_id, signals).await?;
    Ok(Resolution::Created(created))
}

/// A single same-type entity already carrying one of these weak handles is a confident
/// candidate. Name similarity across *different* identities is deliberately NOT auto-merged.
async fn weak_candidate(
    store: &BearMemoryStore,
    entity_type: &str,
    weak: &[&Signal],
    _display_name: Option<&str>,
) -> Result<Option<EntityRow>, DenError> {
    let mut found: Option<EntityRow> = None;
    for sig in weak {
        if let Some(e) =
            entity::find_entity_by_handle(store, &sig.handle_type, &sig.handle_value).await?
        {
            if e.entity_type != entity_type {
                continue;
            }
            match &found {
                Some(prev) if prev.entity_id != e.entity_id => return Ok(None), // ambiguous
                _ => found = Some(e),
            }
        }
    }
    Ok(found)
}

async fn attach_signals(
    store: &BearMemoryStore,
    entity_id: &str,
    signals: &[Signal],
) -> Result<(), DenError> {
    for sig in signals {
        let trust = match sig.strength() {
            HandleStrength::Strong => EntityTrust::Asserted,
            HandleStrength::Weak => EntityTrust::Inferred,
        };
        entity::attach_handle(
            store,
            entity_id,
            &sig.handle_type,
            &sig.handle_value,
            sig.source.as_deref(),
            trust,
        )
        .await?;
    }
    Ok(())
}

fn entry_levels(assertion: Assertion, strong: bool) -> (ResolutionState, EntityTrust) {
    match (assertion, strong) {
        (Assertion::Asserted, _) => (ResolutionState::Confirmed, EntityTrust::Asserted),
        (Assertion::Inferred, true) => (ResolutionState::Resolved, EntityTrust::Inferred),
        (Assertion::Inferred, false) => (ResolutionState::Provisional, EntityTrust::Inferred),
    }
}

/// The work-surface resolver — the first per-type resolver, preserving today's behavior
/// (`git_remote` strong, `checkout` weak) on the shared lifecycle (ADR-0042 §5).
pub async fn resolve_work_surface(
    store: &BearMemoryStore,
    display_name: Option<&str>,
    git_remote: Option<&str>,
    checkout: Option<&str>,
) -> Result<Resolution, DenError> {
    let mut signals = Vec::new();
    if let Some(remote) = git_remote {
        signals.push(Signal::new("git_remote", remote));
    }
    if let Some(path) = checkout {
        signals.push(Signal::new("checkout", path));
    }
    resolve(
        store,
        "work_surface",
        display_name,
        &signals,
        Assertion::Inferred,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::new_test_store;

    #[tokio::test]
    async fn strong_handle_resolves_to_same_entity() {
        let store = new_test_store().await;
        let first = resolve(
            &store,
            "work_surface",
            Some("acme/app"),
            &[Signal::new("git_remote", "github.com/acme/app")],
            Assertion::Inferred,
        )
        .await
        .unwrap();
        let id = match first {
            Resolution::Resolved(e) => e.entity_id,
            other => panic!("expected Resolved, got {other:?}"),
        };
        // A later signal carrying the same strong handle resolves to the same entity.
        let again = resolve(
            &store,
            "work_surface",
            None,
            &[
                Signal::new("git_remote", "github.com/acme/app"),
                Signal::new("checkout", "/home/h/app"),
            ],
            Assertion::Inferred,
        )
        .await
        .unwrap();
        match again {
            Resolution::Resolved(e) => assert_eq!(e.entity_id, id),
            other => panic!("expected Resolved same entity, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn weak_only_stays_provisional() {
        let store = new_test_store().await;
        let outcome = resolve(
            &store,
            "person",
            Some("Ryan"),
            &[Signal::new("mention", "Ryan")],
            Assertion::Inferred,
        )
        .await
        .unwrap();
        match outcome {
            Resolution::Created(e) => {
                assert_eq!(e.resolution, ResolutionState::Provisional);
                assert!(!e.resolution.can_bear_access());
            }
            other => panic!("expected Created provisional, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ryan_multi_handle_does_not_auto_merge() {
        let store = new_test_store().await;
        // Slack Ryan and email Ryan arrive as separate strong identities.
        let slack = resolve(
            &store,
            "person",
            Some("Ryan"),
            &[Signal::new("slack_user", "T01:U07")],
            Assertion::Inferred,
        )
        .await
        .unwrap();
        let email = resolve(
            &store,
            "person",
            Some("Ryan"),
            &[Signal::new("email", "ryan@acme.com")],
            Assertion::Inferred,
        )
        .await
        .unwrap();
        let slack_id = match slack {
            Resolution::Resolved(e) => e.entity_id,
            other => panic!("expected Resolved, got {other:?}"),
        };
        let email_id = match email {
            Resolution::Resolved(e) => e.entity_id,
            other => panic!("expected Resolved, got {other:?}"),
        };
        assert_ne!(
            slack_id, email_id,
            "different strong identities must stay separate"
        );

        // A signal claiming both are one ⇒ conflict for curate, never automatic.
        let conflict = resolve(
            &store,
            "person",
            None,
            &[
                Signal::new("slack_user", "T01:U07"),
                Signal::new("email", "ryan@acme.com"),
            ],
            Assertion::Inferred,
        )
        .await
        .unwrap();
        match conflict {
            Resolution::ConflictNeedsReview { entity_ids } => {
                assert_eq!(entity_ids.len(), 2);
                assert!(entity_ids.contains(&slack_id));
                assert!(entity_ids.contains(&email_id));
            }
            other => panic!("expected ConflictNeedsReview, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_asserted_person_enters_confirmed() {
        let store = new_test_store().await;
        let outcome = resolve(
            &store,
            "person",
            Some("Hans"),
            &[Signal::new("session_human", "acp-token-abc")],
            Assertion::Asserted,
        )
        .await
        .unwrap();
        match outcome {
            Resolution::Resolved(e) => {
                assert_eq!(e.resolution, ResolutionState::Confirmed);
                assert_eq!(e.trust, EntityTrust::Asserted);
            }
            other => panic!("expected Resolved confirmed, got {other:?}"),
        }
    }
}
