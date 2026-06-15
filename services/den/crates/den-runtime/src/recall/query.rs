//! Recall query (ADR-0038 Phase 2): embed a turn query, search the Bear's recall collection,
//! and shape the top hits into passages for the turn assembler's `## Recalled memory` section.
//!
//! Recall is **derived and optional**. Every entry point here is best-effort: an unset
//! `QDRANT_URL`, a disabled embedding client, or any transport error yields *no* recall
//! rather than failing the turn (the canonical key-memory projection still renders).

use serde_json::{json, Value};
use uuid::Uuid;

use den_core::DenError;

use super::indexer::PassageEmbedder;
use super::policy::SOURCE_CLASS_BEAR_MEMORY;
use super::qdrant::QdrantRecall;

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
    pub text: String,
}

/// The outcome of a recall query: the selected passages plus a diagnostic for observability.
#[derive(Debug, Clone)]
pub struct RecallProjection {
    pub passages: Vec<RecalledPassage>,
    pub diagnostic: Value,
}

/// Embed `query_text`, search the Bear's recall collection, dedupe by memory id, and return
/// up to `limit` passages ordered by similarity. Best-effort: errors surface as `Err` so the
/// caller can log + degrade; an empty/blank query returns no passages.
pub async fn recall_for_turn<E: PassageEmbedder + ?Sized>(
    qdrant: &QdrantRecall,
    embedder: &E,
    embedding_standard: &str,
    bear_id: Uuid,
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

    // Scope strictly to this Bear's own memory passages. Access-bearing gating (AccessContext)
    // is a no-op today (no access rules exist yet) and lands with the entity layer (Phase 6).
    let filter = json!({
        "must": [
            { "key": "bear_id", "match": { "value": bear_id.to_string() } },
            { "key": "source_class", "match": { "value": SOURCE_CLASS_BEAR_MEMORY } },
            { "key": "embedding_standard", "match": { "value": embedding_standard } },
        ]
    });

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
            score: hit.score,
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
}
