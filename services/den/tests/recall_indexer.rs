//! Integration test for the derived recall indexer (ADR-0038 Phase 1).
//!
//! Exercises the full write path against a **live Postgres + Qdrant** using a deterministic
//! stub embedder (no OpenAI key required): write memory → passages appear in Qdrant →
//! re-index is idempotent → remove deletes the points.
//!
//! Gated on `DATABASE_URL` + `QDRANT_URL`; skips (passes) when either is unset, so it is a
//! no-op in environments without the recall stack.

use den::{config::Config, startup::run_sqlx_migrations};
use den_runtime::recall::{DeterministicEmbedder, IndexRequest, QdrantRecall, RecallIndexer};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn recall_env_ready() -> bool {
    std::env::var("DATABASE_URL").is_ok() && !std::env::var("QDRANT_URL").unwrap_or_default().is_empty()
}

#[tokio::test]
async fn recall_indexer_round_trip_against_live_qdrant() {
    dotenvy::dotenv().ok();
    if !recall_env_ready() {
        eprintln!("skipping: DATABASE_URL/QDRANT_URL not set");
        return;
    }

    let config = Config::load();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .expect("connect Postgres");
    run_sqlx_migrations(&pool)
        .await
        .expect("apply migrations (recall_passages)");

    let qdrant = QdrantRecall::from_config(&config).expect("QDRANT_URL set");
    qdrant.ensure_collection().await.expect("ensure collection");

    // recall_passages.bear_id has an FK to bears(id); use an existing bear from the DB.
    let bear_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM bears LIMIT 1")
        .fetch_optional(&pool)
        .await
        .expect("query bears");
    let Some(bear_id) = bear_id else {
        eprintln!("skipping: no seeded bear in DB");
        return;
    };

    let embedder = DeterministicEmbedder::new(config.embedding_dimensions);
    let indexer = RecallIndexer::new(&pool, &qdrant, &embedder, config.embedding_standard.clone());

    // Long enough to split into multiple chunks (chunk target is ~2400 chars).
    let body = "Bears keep canonical memory in SQLite. ".repeat(120);
    let memory_id = format!("smoke-recall-{}", Uuid::new_v4());
    let req = IndexRequest {
        bear_id,
        memory_id: memory_id.clone(),
        logical_path: Some("core/recall/smoke.md".into()),
        scope_type: "shared".into(),
        scope_profile: None,
        work_surface_ref: None,
        kind: "summary".into(),
        visibility: "normal".into(),
        content_text: body.clone(),
    };

    let mem_filter = json!({ "must": [{ "key": "memory_id", "match": { "value": memory_id } }] });

    // First index: embeds and upserts every chunk.
    let outcome = indexer.index_record(&req).await.expect("index_record");
    assert!(
        outcome.embedded_chunks >= 2,
        "expected multi-chunk embed, got {outcome:?}"
    );
    assert_eq!(outcome.reused_chunks, 0, "{outcome:?}");

    let count = qdrant
        .count_with_filter(mem_filter.clone())
        .await
        .expect("count after index");
    assert_eq!(
        count as usize, outcome.embedded_chunks,
        "Qdrant should hold one point per embedded chunk"
    );

    // Re-index identical content: fully deduped, no new embeds, count unchanged.
    let again = indexer.index_record(&req).await.expect("re-index");
    assert_eq!(again.embedded_chunks, 0, "{again:?}");
    assert_eq!(again.reused_chunks, outcome.embedded_chunks, "{again:?}");
    let count_after_reindex = qdrant
        .count_with_filter(mem_filter.clone())
        .await
        .expect("count after re-index");
    assert_eq!(count_after_reindex, count, "re-index must not duplicate points");

    // Remove (supersede/delete): points disappear from Qdrant.
    let removed = indexer.remove_record(&req).await.expect("remove_record");
    assert_eq!(removed, outcome.embedded_chunks, "removed all chunk points");
    let count_after_remove = qdrant
        .count_with_filter(mem_filter)
        .await
        .expect("count after remove");
    assert_eq!(count_after_remove, 0, "supersede must remove old passages");
}
