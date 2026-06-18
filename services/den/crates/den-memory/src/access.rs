//! Access-bearing enforcement (ADR-0042 §7, §11): the recall gate.
//!
//! `memory_access_rules` is the only table consulted here, and `AccessContext` is the only
//! way to ask "may the current run see this record?". Every recall/projection read must thread
//! an `AccessContext`; an empty context is **fail-closed** — any record carrying an
//! access-bearing relation is hidden unless the context explicitly grants it. This is what
//! lets access-rule writes (Phase 6) be enabled only after the gate is structurally in place.

use std::collections::BTreeSet;

use den_core::DenError;

use super::descriptors;
use super::entity::resolve_live_entity;
use super::records::BearMemoryStore;
use super::relations::list_access_rules_for_source;

/// The caller's current access context: which entities the Bear is acting with/for
/// (`audience`) and which scopes the run is confined to (`confined_to`).
///
/// Entity ids should be **live** ids; rule targets are resolved through the merge forward
/// pointer before comparison, so a merged audience still matches its survivor.
#[derive(Debug, Clone, Default)]
pub struct AccessContext {
    audience: BTreeSet<String>,
    confinement: BTreeSet<String>,
}

impl AccessContext {
    /// Fail-closed context: grants nothing. Any access-gated record is hidden.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Entities the run is acting with/for — satisfies `audience` rules.
    pub fn with_audience<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.audience.extend(ids.into_iter().map(Into::into));
        self
    }

    /// Scopes the run is confined to — satisfies `confined_to` rules.
    pub fn with_confinement<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.confinement.extend(ids.into_iter().map(Into::into));
        self
    }

    pub fn allow_audience(&mut self, entity_id: impl Into<String>) {
        self.audience.insert(entity_id.into());
    }

    pub fn allow_confinement(&mut self, entity_id: impl Into<String>) {
        self.confinement.insert(entity_id.into());
    }

    pub fn is_empty(&self) -> bool {
        self.audience.is_empty() && self.confinement.is_empty()
    }
}

/// Whether `src_memory_id` may be surfaced under `ctx`.
///
/// - No access-bearing relations ⇒ always visible.
/// - `audience` rules ⇒ the context must include **at least one** addressed entity (a note
///   addressed to several people is visible when acting with any of them).
/// - `confined_to` rules ⇒ the context must include **every** confinement scope (hard
///   non-leak: a record confined to two scopes only surfaces inside both).
/// - Both classes must pass.
pub async fn record_visible(
    store: &BearMemoryStore,
    src_memory_id: &str,
    ctx: &AccessContext,
) -> Result<bool, DenError> {
    let rules = list_access_rules_for_source(store, src_memory_id).await?;
    if rules.is_empty() {
        return Ok(true);
    }

    let mut audience_targets: BTreeSet<String> = BTreeSet::new();
    let mut confinement_targets: BTreeSet<String> = BTreeSet::new();

    for rule in &rules {
        // Resolve the rule's target through the merge forward pointer to the live survivor.
        let live_id = match resolve_live_entity(store, &rule.entity_id).await? {
            Some(e) => e.entity_id,
            None => rule.entity_id.clone(),
        };
        match descriptors::relation(&rule.relation).map(|d| d.alias) {
            Some("audience") => {
                audience_targets.insert(live_id);
            }
            Some("confined_to") => {
                confinement_targets.insert(live_id);
            }
            // Unknown access-bearing relation: fail-closed (treat as a confinement we can't satisfy).
            _ => {
                confinement_targets.insert(live_id);
            }
        }
    }

    if !audience_targets.is_empty() && audience_targets.is_disjoint(&ctx.audience) {
        return Ok(false);
    }
    if !confinement_targets.is_empty() && !confinement_targets.is_subset(&ctx.confinement) {
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::merge_entities;
    use crate::records::BearMemoryStore;
    use crate::relations::append_relation;
    use crate::resolver::{resolve, Assertion, Resolution, Signal};
    use crate::test_support::new_test_store;
    use serde_json::json;

    async fn confirmed_person(store: &BearMemoryStore, name: &str, email: &str) -> String {
        match resolve(
            store,
            "person",
            Some(name),
            &[Signal::new("email", email)],
            Assertion::Asserted,
        )
        .await
        .unwrap()
        {
            Resolution::Resolved(e) => e.entity_id,
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    async fn confirmed_surface(store: &BearMemoryStore, remote: &str) -> String {
        match resolve(
            store,
            "work_surface",
            Some("surface"),
            &[Signal::new("git_remote", remote)],
            Assertion::Asserted,
        )
        .await
        .unwrap()
        {
            Resolution::Resolved(e) => e.entity_id,
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn record_without_access_rules_is_always_visible() {
        let store = new_test_store().await;
        assert!(record_visible(&store, "mem-open", &AccessContext::empty())
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn audience_gate_hides_unless_context_includes_addressee() {
        let store = new_test_store().await;
        let alice = confirmed_person(&store, "Alice", "alice@acme.com").await;
        let bob = confirmed_person(&store, "Bob", "bob@acme.com").await;
        append_relation(&store, "mem-a", &alice, "audience", &json!({}), "curate", None, None)
            .await
            .unwrap();

        // Empty context is fail-closed.
        assert!(!record_visible(&store, "mem-a", &AccessContext::empty())
            .await
            .unwrap());
        // Acting with Alice grants it.
        assert!(record_visible(
            &store,
            "mem-a",
            &AccessContext::empty().with_audience([alice.clone()])
        )
        .await
        .unwrap());
        // Acting with Bob does not.
        assert!(!record_visible(
            &store,
            "mem-a",
            &AccessContext::empty().with_audience([bob])
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn confined_to_prevents_cross_surface_leak() {
        let store = new_test_store().await;
        let client_a = confirmed_surface(&store, "github.com/acme/client-a").await;
        let client_b = confirmed_surface(&store, "github.com/acme/client-b").await;
        append_relation(
            &store,
            "mem-c",
            &client_a,
            "confined_to",
            &json!({}),
            "curate",
            None,
            None,
        )
        .await
        .unwrap();

        assert!(!record_visible(&store, "mem-c", &AccessContext::empty())
            .await
            .unwrap());
        assert!(record_visible(
            &store,
            "mem-c",
            &AccessContext::empty().with_confinement([client_a.clone()])
        )
        .await
        .unwrap());
        // Inside client-B's surface, client-A memory stays hidden.
        assert!(!record_visible(
            &store,
            "mem-c",
            &AccessContext::empty().with_confinement([client_b])
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn both_classes_must_pass() {
        let store = new_test_store().await;
        let alice = confirmed_person(&store, "Alice", "alice@acme.com").await;
        let surface = confirmed_surface(&store, "github.com/acme/app").await;
        append_relation(&store, "mem-d", &alice, "audience", &json!({}), "curate", None, None)
            .await
            .unwrap();
        append_relation(
            &store,
            "mem-d",
            &surface,
            "confined_to",
            &json!({}),
            "curate",
            None,
            None,
        )
        .await
        .unwrap();

        // Audience-only context: confinement check still fails.
        assert!(!record_visible(
            &store,
            "mem-d",
            &AccessContext::empty().with_audience([alice.clone()])
        )
        .await
        .unwrap());
        // Both satisfied.
        assert!(record_visible(
            &store,
            "mem-d",
            &AccessContext::empty()
                .with_audience([alice])
                .with_confinement([surface])
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn audience_follows_merge_forward_pointer() {
        let store = new_test_store().await;
        let survivor = confirmed_person(&store, "Alice", "alice@acme.com").await;
        let loser = confirmed_person(&store, "Alice (dup)", "alice@personal.com").await;
        // Address the record to the (soon-to-be-merged) duplicate.
        append_relation(&store, "mem-e", &loser, "audience", &json!({}), "curate", None, None)
            .await
            .unwrap();
        merge_entities(&store, &survivor, &loser).await.unwrap();

        // Context carrying the survivor id resolves the merged audience.
        assert!(record_visible(
            &store,
            "mem-e",
            &AccessContext::empty().with_audience([survivor])
        )
        .await
        .unwrap());
    }
}
