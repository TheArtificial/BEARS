//! Recall query (ADR-0038 Phase 2): embed a turn query, search the Bear's recall collection,
//! and shape the top hits into passages for the turn assembler's `## Recalled memory` section.
//!
//! Recall is **derived and optional**. Every entry point here is best-effort: an unset
//! `QDRANT_URL`, a disabled embedding client, or any transport error yields *no* recall
//! rather than failing the turn (the canonical key-memory projection still renders).

use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::{config::Config, DenError};

use super::indexer::PassageEmbedder;
use super::policy::SOURCE_CLASS_BEAR_MEMORY;
use super::qdrant::QdrantRecall;
use super::temporal::{parse_time_expression, TemporalQuery};

/// Total character budget for the rendered recall section (ADR-0038 Phase 2: ~2–3k).
const RECALL_CHAR_BUDGET: usize = 2_600;
/// Max characters of a single passage snippet before truncation.
const SNIPPET_CHARS: usize = 480;

/// A single recalled passage, resolved from a Qdrant hit's payload.
#[derive(Debug, Clone)]
pub struct RecalledPassage {
    pub memory_id: String,
    pub logical_path: Option<String>,
    pub kind: Option<String>,
    pub score: f32,
    pub salience: String,
    pub text: String,
}

fn salience_multiplier(salience: &str) -> f32 {
    match salience {
        "low" => 0.9,
        "high" => 1.15,
        "critical" => 1.3,
        _ => 1.0,
    }
}

/// The outcome of a recall query: the selected passages plus a diagnostic for observability.
#[derive(Debug, Clone)]
pub struct RecallProjection {
    pub passages: Vec<RecalledPassage>,
    pub diagnostic: Value,
}

/// An empty projection tagged `disabled` (never an error) so config-driven callers can fall
/// back to keyword search when recall isn't fully wired (`QDRANT_URL` / embeddings unset).
fn disabled_projection(reason: &str) -> RecallProjection {
    RecallProjection {
        passages: Vec::new(),
        diagnostic: json!({ "source": "recall_query", "status": "disabled", "reason": reason }),
    }
}

/// Mandatory filter conditions scoping a search to this Bear's own memory passages under the
/// active embedding standard. Access-bearing gating (AccessContext) is a no-op today (no access
/// rules exist yet) and lands with the entity layer (Phase 6).
fn bear_scope_conditions(bear_id: Uuid, embedding_standard: &str) -> Vec<Value> {
    vec![
        json!({ "key": "bear_id", "match": { "value": bear_id.to_string() } }),
        json!({ "key": "source_class", "match": { "value": SOURCE_CLASS_BEAR_MEMORY } }),
        json!({ "key": "embedding_standard", "match": { "value": embedding_standard } }),
    ]
}

/// Filter scoping a search to memory **visible to `role`**: shared (core) records OR this role's
/// own profile-local records. Other roles' profile-local memory is excluded, honoring the
/// role-local boundary (AGENTS.md: `work` must not read raw `pair/`). The nested `should`
/// (≥1 match) acts as an OR clause within the mandatory bear scope.
fn role_scope_filter(bear_id: Uuid, embedding_standard: &str, role: &str) -> Value {
    let mut must = bear_scope_conditions(bear_id, embedding_standard);
    must.push(json!({
        "should": [
            { "key": "scope_type", "match": { "value": "shared" } },
            { "key": "scope_profile", "match": { "value": role } },
        ]
    }));
    json!({ "must": must })
}

/// Mandatory bear-scope conditions plus an **entity-membership** clause: passages whose
/// denormalized `entity_ids` array contains *any* of `entity_ids` (ADR-0042 §7 descriptive
/// relations; the access-bearing gate is never denormalized here). The empty-`entity_ids` case is
/// handled by callers (no entities ⇒ no scope ⇒ skip). Bear-wide; a turn-time caller layers role
/// scoping on top.
fn entity_scope_filter(bear_id: Uuid, embedding_standard: &str, entity_ids: &[String]) -> Value {
    let mut must = bear_scope_conditions(bear_id, embedding_standard);
    must.push(json!({ "key": "entity_ids", "match": { "any": entity_ids } }));
    json!({ "must": must })
}

/// Embed `query_text`, run a `filter`-scoped Qdrant search, and dedupe to the best-scoring chunk
/// per memory id, returning up to `limit` passages ordered by similarity. Best-effort: errors
/// surface as `Err` so the caller can log + degrade; an empty/blank query returns no passages.
async fn search_passages<E: PassageEmbedder + ?Sized>(
    qdrant: &QdrantRecall,
    embedder: &E,
    filter: Value,
    embedding_standard: &str,
    query_text: &str,
    limit: usize,
) -> Result<RecallProjection, DenError> {
    let trimmed = query_text.trim();
    if trimmed.is_empty() || limit == 0 {
        return Ok(RecallProjection {
            passages: Vec::new(),
            diagnostic: json!({
                "source": "recall_query",
                "status": "skipped",
                "reason": "empty_query",
            }),
        });
    }

    let vectors = embedder.embed(std::slice::from_ref(&trimmed.to_string())).await?;
    let query_vec = vectors
        .into_iter()
        .next()
        .ok_or_else(|| DenError::System("recall query embedding returned no vector".to_string()))?;

    // Overfetch so per-memory dedupe still yields `limit` distinct records.
    let fetch = (limit.saturating_mul(3)).clamp(limit, 50) as u64;
    let hits = qdrant.search(&query_vec, filter, fetch).await?;
    let raw_hits = hits.len();

    let mut passages: Vec<RecalledPassage> = Vec::new();
    for hit in hits {
        let memory_id = hit
            .payload
            .get("memory_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let Some(memory_id) = memory_id else { continue };
        // Keep only the best-scoring chunk per memory record (hits arrive best-first).
        if passages.iter().any(|p| p.memory_id == memory_id) {
            continue;
        }
        let text = hit
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let salience = hit
            .payload
            .get("salience")
            .and_then(Value::as_str)
            .unwrap_or("normal")
            .to_string();
        passages.push(RecalledPassage {
            memory_id,
            logical_path: hit
                .payload
                .get("logical_path")
                .and_then(Value::as_str)
                .map(str::to_string),
            kind: hit
                .payload
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_string),
            score: hit.score * salience_multiplier(&salience),
            salience,
            text,
        });
        if passages.len() >= limit {
            break;
        }
    }

    let diagnostic = json!({
        "source": "recall_query",
        "status": "ok",
        "raw_hits": raw_hits,
        "passages": passages.len(),
        "embedding_standard": embedding_standard,
    });
    Ok(RecallProjection { passages, diagnostic })
}

/// Embed `query_text`, search the Bear's recall collection (bear-wide), dedupe by memory id, and
/// return up to `limit` passages. Used by the turn assembler's `## Recalled memory` section.
pub async fn recall_for_turn<E: PassageEmbedder + ?Sized>(
    qdrant: &QdrantRecall,
    embedder: &E,
    embedding_standard: &str,
    bear_id: Uuid,
    query_text: &str,
    limit: usize,
) -> Result<RecallProjection, DenError> {
    let filter = json!({ "must": bear_scope_conditions(bear_id, embedding_standard) });
    search_passages(qdrant, embedder, filter, embedding_standard, query_text, limit).await
}

/// Role-scoped turn recall: like [`recall_for_turn`] but limited to memory **visible to `role`**
/// (shared + own role-local), so the assembler's `## Recalled memory` section honors the same
/// role-local boundary as the `memory_search` tool (AGENTS.md: `work` must not read raw `pair/`).
pub async fn recall_for_turn_scoped<E: PassageEmbedder + ?Sized>(
    qdrant: &QdrantRecall,
    embedder: &E,
    embedding_standard: &str,
    bear_id: Uuid,
    role: &str,
    query_text: &str,
    limit: usize,
) -> Result<RecallProjection, DenError> {
    let filter = role_scope_filter(bear_id, embedding_standard, role);
    search_passages(qdrant, embedder, filter, embedding_standard, query_text, limit).await
}

/// Convenience **bear-wide** semantic search for the admin UI (the human admin sees all of a
/// Bear's memory): builds the live Qdrant + Bifrost embedding clients from config. Returns a
/// `disabled`-tagged projection (never an error) when recall isn't fully configured.
pub async fn semantic_search_for_bear(
    config: &Config,
    bear_id: Uuid,
    query_text: &str,
    limit: usize,
) -> Result<RecallProjection, DenError> {
    let Some(qdrant) = QdrantRecall::from_config(config) else {
        return Ok(disabled_projection("qdrant_unset"));
    };
    let embedder = den_llm::EmbeddingClient::new(config);
    if !embedder.is_enabled() {
        return Ok(disabled_projection("embeddings_unset"));
    }
    let filter = json!({ "must": bear_scope_conditions(bear_id, &config.embedding_standard) });
    search_passages(
        &qdrant,
        &embedder,
        filter,
        &config.embedding_standard,
        query_text,
        limit,
    )
    .await
}

/// **Role-scoped** semantic search for the `memory_search` tool: scopes to memory visible to
/// `role` (shared + own role-local). Builds the live Qdrant + Bifrost embedding clients from
/// config and returns a `disabled`-tagged projection (never an error) when recall isn't
/// configured, so the tool can fall back to keyword search.
pub async fn search_bear_memory_for_role(
    config: &Config,
    bear_id: Uuid,
    role: &str,
    query_text: &str,
    limit: usize,
) -> Result<RecallProjection, DenError> {
    let Some(qdrant) = QdrantRecall::from_config(config) else {
        return Ok(disabled_projection("qdrant_unset"));
    };
    let embedder = den_llm::EmbeddingClient::new(config);
    if !embedder.is_enabled() {
        return Ok(disabled_projection("embeddings_unset"));
    }
    let filter = role_scope_filter(bear_id, &config.embedding_standard, role);
    search_passages(
        &qdrant,
        &embedder,
        filter,
        &config.embedding_standard,
        query_text,
        limit,
    )
    .await
}

/// **Entity-scoped** semantic search (ADR-0042 Phase 4 recall leg): rank the Bear's passages that
/// are linked by a descriptive relation to any of `entity_ids`, by relevance to `query_text`.
/// This is the query-side consumer of the denormalized passage `entity_ids` and the seed leg for
/// future bounded-graph expansion + entity-centric admin recall. Bear-wide (the human admin sees
/// all). Builds live Qdrant + Bifrost embedding clients from config; returns a `disabled`/`skipped`
/// projection (never an error) when recall isn't configured or no entities are supplied.
pub async fn search_bear_memory_for_entities(
    config: &Config,
    bear_id: Uuid,
    entity_ids: &[String],
    query_text: &str,
    limit: usize,
) -> Result<RecallProjection, DenError> {
    if entity_ids.is_empty() {
        return Ok(disabled_projection("no_entities"));
    }
    let Some(qdrant) = QdrantRecall::from_config(config) else {
        return Ok(disabled_projection("qdrant_unset"));
    };
    let embedder = den_llm::EmbeddingClient::new(config);
    if !embedder.is_enabled() {
        return Ok(disabled_projection("embeddings_unset"));
    }
    let filter = entity_scope_filter(bear_id, &config.embedding_standard, entity_ids);
    search_passages(
        &qdrant,
        &embedder,
        filter,
        &config.embedding_standard,
        query_text,
        limit,
    )
    .await
}

/// Default hop cap for the bounded-graph recall leg (ADR-0042 §4 / DERIVED_RECALL Phase 3.5).
const GRAPH_MAX_DEPTH: u32 = 2;

/// Bounded-graph recall leg: expand from `seed_memory_ids` over the descriptive record↔entity
/// graph (depth `max_depth`) and render the newly reached records as role-scoped hits, closest
/// hops first. Read-only, retrieval-time; no stored edges/inference (ADR-0042 anti-RDF). Surfaces
/// records related through a shared entity that the vector/keyword legs never matched. Reached
/// records are role-gated by `fetch_records_min` (shared ∨ own role-local); access-bearing gating
/// is inherent (traversal is over the descriptive table only) and `AccessContext` is applied by
/// the caller once access rules exist.
pub async fn graph_expand_hits(
    config: &Config,
    bear_id: Uuid,
    role: &str,
    seed_memory_ids: &[String],
    max_depth: u32,
    limit: usize,
) -> Result<Vec<Value>, DenError> {
    if seed_memory_ids.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let stores = den_memory::MemoryStoreManager::new(config);
    let store = stores.store_for_bear(bear_id).await?;
    let reached =
        den_memory::bounded_graph_expand(&store, seed_memory_ids, max_depth, limit)
            .await?;
    if reached.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = reached.iter().map(|r| r.memory_id.clone()).collect();
    let records = den_memory::fetch_records_min(&store, &ids, role).await?;
    let by_id: std::collections::HashMap<&str, &den_memory::RecallRecordMin> =
        records.iter().map(|r| (r.memory_id.as_str(), r)).collect();

    // Entity-overlap boost: rank reached records that share more entities with the seed set higher
    // within each hop tier (a stronger association than a single shared entity).
    let seed_entities: std::collections::HashSet<String> =
        den_memory::relations::descriptive_entity_ids_for_records(&store, seed_memory_ids)
            .await?
            .into_iter()
            .collect();
    let entities_by_record =
        den_memory::relations::descriptive_entity_ids_by_source(&store).await?;
    let overlap = |memory_id: &str| -> usize {
        entities_by_record
            .get(memory_id)
            .map(|entities| entities.iter().filter(|e| seed_entities.contains(*e)).count())
            .unwrap_or(0)
    };

    // Role-visible reached records, ordered by hop asc, then overlap desc, then id (determinism).
    let mut ordered: Vec<(&den_memory::GraphReach, usize)> = reached
        .iter()
        .filter(|reach| by_id.contains_key(reach.memory_id.as_str()))
        .map(|reach| {
            let score = overlap(&reach.memory_id);
            (reach, score)
        })
        .collect();
    ordered.sort_by(|(a, oa), (b, ob)| {
        a.hop
            .cmp(&b.hop)
            .then(ob.cmp(oa))
            .then(a.memory_id.cmp(&b.memory_id))
    });

    let hits = ordered
        .into_iter()
        .map(|(reach, entity_overlap)| {
            let rec = by_id[reach.memory_id.as_str()];
            json!({
                "memory_id": rec.memory_id,
                "path": rec.logical_path,
                "kind": rec.kind,
                "score": Value::Null,
                "snippet": truncate_chars(&rec.content_text, SNIPPET_CHARS),
                "salience": rec.salience,
                "source": "graph",
                "hop": reach.hop,
                "entity_overlap": entity_overlap,
            })
        })
        .collect();
    Ok(hits)
}

/// Hybrid `memory_search` (ADR-0038 Phase 3 + 3.5): the **union** of three role-scoped legs over
/// the same visibility — the derived **vector** index, the **keyword** (`LIKE`) leg over canonical
/// SQLite, and the bounded-**graph** leg that expands from the direct hits over the record↔entity
/// relation graph. Vector hits rank first (higher-signal, carry a `score`), then keyword-only
/// exact matches, then graph-reached records (relevant by association, never directly matched).
///
/// Every leg beyond keyword is best-effort: an unconfigured index/transport error (vector) or a
/// relation-store error (graph) degrades to the remaining legs (logged), never failing the tool.
/// The keyword leg reads the canonical store, so its errors propagate.
pub async fn hybrid_memory_search(
    config: &Config,
    bear_id: Uuid,
    role: &str,
    query: &str,
    limit: usize,
) -> Result<Value, DenError> {
    // Temporal leg (Phase 3.5): split a time expression off the query so the direct legs match on
    // the topical remainder while recall filters/boosts on effective event time.
    let temporal = parse_time_expression(query, OffsetDateTime::now_utc());
    let effective_query = temporal
        .as_ref()
        .map(|t| t.residual_query.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(query);
    // Over-fetch when a temporal filter will prune, so enough in-window hits survive the cap.
    let fetch_limit = if temporal.is_some() {
        limit.saturating_mul(5).clamp(limit, 200)
    } else {
        limit
    };

    let vector =
        match search_bear_memory_for_role(config, bear_id, role, effective_query, fetch_limit).await
        {
            Ok(projection) => projection,
            Err(error) => {
                tracing::warn!(%error, "recall vector leg failed; returning keyword results only");
                disabled_projection("vector_error")
            }
        };

    let stores = den_memory::MemoryStoreManager::new(config);
    let store = stores.store_for_bear(bear_id).await?;
    let limit_i64 = i64::try_from(fetch_limit).unwrap_or(10);
    let keyword =
        den_memory::tools::sqlite_memory_search(&store, role, effective_query, limit_i64).await?;

    // Seed the graph leg with the records the direct legs already matched.
    let mut seeds: Vec<String> = vector.passages.iter().map(|p| p.memory_id.clone()).collect();
    if let Some(arr) = keyword.get("hits").and_then(Value::as_array) {
        for hit in arr {
            if let Some(id) = hit.get("memory_id").and_then(Value::as_str) {
                seeds.push(id.to_string());
            }
        }
    }
    let graph =
        match graph_expand_hits(config, bear_id, role, &seeds, GRAPH_MAX_DEPTH, fetch_limit).await {
            Ok(hits) => hits,
            Err(error) => {
                tracing::warn!(%error, "recall graph leg failed; returning direct results only");
                Vec::new()
            }
        };

    let mut result = merge_search_results(&vector, &keyword, &graph, query, fetch_limit);

    if let Some(temporal) = &temporal {
        let hits = result
            .get("hits")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut filtered = match filter_hits_by_temporal(&store, hits, temporal).await {
            Ok(hits) => hits,
            Err(error) => {
                tracing::warn!(%error, "recall temporal leg failed; returning unfiltered results");
                Vec::new()
            }
        };
        filtered.truncate(limit);
        result["hits"] = Value::Array(filtered);
        result["temporal"] = json!({
            "matched": temporal.matched,
            "as_of": temporal.as_of,
            "from": temporal.from.and_then(|d| d.format(&Rfc3339).ok()),
            "to": temporal.to.and_then(|d| d.format(&Rfc3339).ok()),
        });
    }

    Ok(result)
}

/// Drop hits whose effective event time (`COALESCE(valid_from, created_at)`) falls outside the
/// parsed window. For point-in-time (`as of`) queries, also drop records already superseded as of
/// the upper bound, walking the supersession chain (Phase 3.5).
async fn filter_hits_by_temporal(
    store: &den_memory::BearMemoryStore,
    hits: Vec<Value>,
    temporal: &TemporalQuery,
) -> Result<Vec<Value>, DenError> {
    if hits.is_empty() {
        return Ok(hits);
    }
    let ids: Vec<String> = hits
        .iter()
        .filter_map(|h| h.get("memory_id").and_then(Value::as_str).map(String::from))
        .collect();
    let times = den_memory::effective_time_by_ids(store, &ids).await?;
    let superseders = if temporal.as_of {
        den_memory::superseder_times_by_superseded(store, &ids).await?
    } else {
        std::collections::HashMap::new()
    };

    Ok(hits
        .into_iter()
        .filter(|hit| {
            let Some(id) = hit.get("memory_id").and_then(Value::as_str) else {
                return false;
            };
            // No effective time ⇒ can't satisfy a temporal constraint.
            let Some(effective) = times.get(id) else {
                return false;
            };
            if let Some(from) = temporal.from {
                if *effective < from {
                    return false;
                }
            }
            if let Some(to) = temporal.to {
                if *effective >= to {
                    return false;
                }
                if temporal.as_of {
                    if let Some(sup_times) = superseders.get(id) {
                        if sup_times.iter().any(|st| *st < to) {
                            return false;
                        }
                    }
                }
            }
            true
        })
        .collect())
}

/// Merge the vector projection, the keyword leg's JSON, and the bounded-graph leg's hits into the
/// unified `memory_search` result. De-dupes by `memory_id` in priority order (vector ≻ keyword ≻
/// graph — a record surfaced by a higher-signal leg is not repeated), caps to `limit`, and tags
/// each hit's `source` plus a top-level `strategy` (the `+`-joined list of contributing legs, e.g.
/// `vector+keyword+graph`, or a single leg, or `none`).
fn merge_search_results(
    vector: &RecallProjection,
    keyword: &Value,
    graph: &[Value],
    query: &str,
    limit: usize,
) -> Value {
    let mut hits: Vec<Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for passage in &vector.passages {
        seen.insert(passage.memory_id.clone());
        hits.push(json!({
            "memory_id": passage.memory_id,
            "path": passage.logical_path,
            "kind": passage.kind,
            "score": passage.score,
            "salience": passage.salience,
            "snippet": truncate_chars(&passage.text, SNIPPET_CHARS),
            "source": "vector",
        }));
    }
    let vector_count = hits.len();

    let mut keyword_count = 0usize;
    if let Some(arr) = keyword.get("hits").and_then(Value::as_array) {
        for hit in arr {
            let Some(memory_id) = hit.get("memory_id").and_then(Value::as_str) else {
                continue;
            };
            if !seen.insert(memory_id.to_string()) {
                continue;
            }
            keyword_count += 1;
            hits.push(json!({
                "memory_id": memory_id,
                "path": hit.get("path").cloned().unwrap_or(Value::Null),
                "kind": hit.get("kind").cloned().unwrap_or(Value::Null),
                "score": Value::Null,
                "snippet": hit.get("snippet").cloned().unwrap_or(Value::Null),
                "salience": hit.get("salience").cloned().unwrap_or_else(|| json!("normal")),
                "source": "keyword",
            }));
        }
    }

    let mut graph_count = 0usize;
    for hit in graph {
        let Some(memory_id) = hit.get("memory_id").and_then(Value::as_str) else {
            continue;
        };
        if !seen.insert(memory_id.to_string()) {
            continue;
        }
        graph_count += 1;
        hits.push(hit.clone());
    }
    hits.truncate(limit);

    let mut legs: Vec<&str> = Vec::new();
    if vector_count > 0 {
        legs.push("vector");
    }
    if keyword_count > 0 {
        legs.push("keyword");
    }
    if graph_count > 0 {
        legs.push("graph");
    }
    let strategy = if legs.is_empty() {
        "none".to_string()
    } else {
        legs.join("+")
    };
    json!({
        "ok": true,
        "configured": true,
        "storage": "hybrid",
        "strategy": strategy,
        "query": query,
        "hits": hits,
        "diagnostic": vector.diagnostic,
    })
}

/// Render the `## Recalled memory` section, dropping passages whose `logical_path` already
/// appears in `anchor_text` (the key-memory projection) so recall never duplicates anchors.
/// Returns `None` when nothing survives dedupe/budget.
pub fn render_recall_block(projection: &RecallProjection, anchor_text: &str) -> Option<String> {
    let mut body = String::new();
    let mut used = 0usize;
    let mut rendered = 0usize;
    for passage in &projection.passages {
        // Dedupe against anchors already injected by the key-memory projection.
        if let Some(path) = passage.logical_path.as_deref() {
            if !path.is_empty() && anchor_text.contains(path) {
                continue;
            }
        }
        let label = passage
            .logical_path
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or(&passage.memory_id);
        let kind = passage.kind.as_deref().unwrap_or("memory");
        let snippet = truncate_chars(&passage.text, SNIPPET_CHARS);
        let line = format!(
            "- `{label}` ({kind}, score {:.2}): {snippet}\n",
            passage.score
        );
        if used + line.len() > RECALL_CHAR_BUDGET && rendered > 0 {
            break;
        }
        used += line.len();
        body.push_str(&line);
        rendered += 1;
    }

    if rendered == 0 {
        return None;
    }
    Some(format!(
        "## Recalled memory\n\nSemantically related memory (derived recall; lower precision than projected anchors above):\n\n{body}"
    ))
}

fn truncate_chars(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(max).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(memory_id: &str, path: &str, score: f32) -> RecalledPassage {
        RecalledPassage {
            memory_id: memory_id.into(),
            logical_path: Some(path.into()),
            kind: Some("note".into()),
            score,
            salience: "normal".into(),
            text: "the quick brown fox jumps over the lazy dog".into(),
        }
    }

    #[test]
    fn render_drops_paths_already_in_anchors() {
        let projection = RecallProjection {
            passages: vec![
                passage("m1", "core/a.md", 0.91),
                passage("m2", "core/b.md", 0.80),
            ],
            diagnostic: Value::Null,
        };
        let anchors = "# Projected memory\n- core/a.md: ...";
        let block = render_recall_block(&projection, anchors).expect("block");
        assert!(block.contains("core/b.md"));
        assert!(!block.contains("core/a.md"));
    }

    #[test]
    fn render_none_when_all_deduped() {
        let projection = RecallProjection {
            passages: vec![passage("m1", "core/a.md", 0.91)],
            diagnostic: Value::Null,
        };
        assert!(render_recall_block(&projection, "core/a.md").is_none());
    }

    #[test]
    fn truncate_collapses_whitespace_and_caps_length() {
        let out = truncate_chars("a\n\n  b   c", 100);
        assert_eq!(out, "a b c");
        let long = "x".repeat(600);
        let capped = truncate_chars(&long, SNIPPET_CHARS);
        assert_eq!(capped.chars().count(), SNIPPET_CHARS + 1); // + ellipsis
    }

    #[test]
    fn role_scope_filter_requires_shared_or_own_role() {
        let bear = Uuid::nil();
        let filter = role_scope_filter(bear, "bears-embed-v1", "work");
        let must = filter["must"].as_array().expect("must array");
        // Three mandatory scope conditions + one nested should clause.
        assert_eq!(must.len(), 4, "{filter}");
        assert_eq!(must[0]["key"], "bear_id");
        assert_eq!(must[1]["match"]["value"], SOURCE_CLASS_BEAR_MEMORY);
        assert_eq!(must[2]["match"]["value"], "bears-embed-v1");
        let should = must[3]["should"].as_array().expect("nested should");
        assert_eq!(should[0]["key"], "scope_type");
        assert_eq!(should[0]["match"]["value"], "shared");
        assert_eq!(should[1]["key"], "scope_profile");
        assert_eq!(should[1]["match"]["value"], "work");
    }

    #[test]
    fn entity_scope_filter_requires_entity_membership() {
        let bear = Uuid::nil();
        let filter = entity_scope_filter(bear, "bears-embed-v1", &["e1".into(), "e2".into()]);
        let must = filter["must"].as_array().expect("must array");
        // Three mandatory scope conditions + one entity-membership clause.
        assert_eq!(must.len(), 4, "{filter}");
        assert_eq!(must[0]["key"], "bear_id");
        assert_eq!(must[3]["key"], "entity_ids");
        let any = must[3]["match"]["any"].as_array().expect("any array");
        assert_eq!(any.len(), 2);
        assert_eq!(any[0], "e1");
        assert_eq!(any[1], "e2");
    }

    #[test]
    fn merge_unions_vector_then_keyword_and_dedupes() {
        let vector = RecallProjection {
            passages: vec![passage("m1", "core/a.md", 0.91)],
            diagnostic: json!({ "status": "ok" }),
        };
        // Keyword leg re-surfaces m1 (must be deduped) and adds a unique m2.
        let keyword = json!({
            "hits": [
                { "memory_id": "m1", "path": "core/a.md", "kind": "note", "snippet": "dup" },
                { "memory_id": "m2", "path": "work/b.md", "kind": "note", "snippet": "exact match" },
            ]
        });
        let value = merge_search_results(&vector, &keyword, &[], "fox", 10);

        assert_eq!(value["storage"], "hybrid");
        assert_eq!(value["strategy"], "vector+keyword");
        let hits = value["hits"].as_array().expect("hits array");
        assert_eq!(hits.len(), 2, "m1 deduped, m2 appended: {hits:?}");
        // Vector hit ranks first and carries its score + source.
        assert_eq!(hits[0]["memory_id"], "m1");
        assert_eq!(hits[0]["source"], "vector");
        assert!((hits[0]["score"].as_f64().unwrap() - 0.91).abs() < 1e-6);
        assert!(hits[0]["snippet"].as_str().unwrap().contains("quick brown fox"));
        // Keyword-only hit follows, unranked.
        assert_eq!(hits[1]["memory_id"], "m2");
        assert_eq!(hits[1]["source"], "keyword");
        assert!(hits[1]["score"].is_null());
    }

    #[test]
    fn merge_strategy_reflects_contributing_legs() {
        let only_vector = merge_search_results(
            &RecallProjection { passages: vec![passage("m1", "core/a.md", 0.9)], diagnostic: Value::Null },
            &json!({ "hits": [] }),
            &[],
            "q",
            10,
        );
        assert_eq!(only_vector["strategy"], "vector");

        let only_keyword = merge_search_results(
            &RecallProjection { passages: vec![], diagnostic: Value::Null },
            &json!({ "hits": [{ "memory_id": "m9", "path": "p", "kind": "note", "snippet": "s" }] }),
            &[],
            "q",
            10,
        );
        assert_eq!(only_keyword["strategy"], "keyword");
    }

    #[test]
    fn merge_appends_graph_leg_after_direct_hits_and_dedupes() {
        let vector = RecallProjection {
            passages: vec![passage("m1", "core/a.md", 0.9)],
            diagnostic: Value::Null,
        };
        let keyword = json!({ "hits": [{ "memory_id": "m2", "path": "work/b.md", "kind": "note", "snippet": "kw" }] });
        // m1 is re-surfaced by the graph leg (must dedupe); m3 is a genuine 2-hop reach.
        let graph = vec![
            json!({ "memory_id": "m1", "path": "core/a.md", "kind": "note", "score": Value::Null, "snippet": "dup", "source": "graph", "hop": 1 }),
            json!({ "memory_id": "m3", "path": "core/c.md", "kind": "note", "score": Value::Null, "snippet": "reached via shared entity", "source": "graph", "hop": 2 }),
        ];
        let value = merge_search_results(&vector, &keyword, &graph, "q", 10);

        assert_eq!(value["strategy"], "vector+keyword+graph");
        let hits = value["hits"].as_array().expect("hits array");
        assert_eq!(hits.len(), 3, "m1 deduped against the vector hit: {hits:?}");
        assert_eq!(hits[0]["memory_id"], "m1");
        assert_eq!(hits[1]["memory_id"], "m2");
        // Graph-reached record ranks last and carries its provenance + hop distance.
        assert_eq!(hits[2]["memory_id"], "m3");
        assert_eq!(hits[2]["source"], "graph");
        assert_eq!(hits[2]["hop"], 2);
        assert!(hits[2]["score"].is_null());
    }

    #[test]
    fn merge_caps_to_limit_prioritizing_vector() {
        let vector = RecallProjection {
            passages: vec![passage("m1", "core/a.md", 0.9), passage("m2", "core/b.md", 0.8)],
            diagnostic: Value::Null,
        };
        let keyword = json!({ "hits": [{ "memory_id": "m3", "path": "p", "kind": "note", "snippet": "s" }] });
        let value = merge_search_results(&vector, &keyword, &[], "q", 2);
        let hits = value["hits"].as_array().expect("hits");
        assert_eq!(hits.len(), 2, "capped to limit");
        assert_eq!(hits[0]["memory_id"], "m1");
        assert_eq!(hits[1]["memory_id"], "m2", "vector hits retained over keyword when capping");
    }
}
