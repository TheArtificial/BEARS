//! Bridge test for `den_runtime::reflection::conversations`. It runs DB migrations
//! via `sqlx::migrate!("./migrations")` (the `den` crate owns the migrations dir),
//! so it lives in the `den` crate rather than in `den-runtime`.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use time::Date;
use uuid::Uuid;

use crate::core::reflection_conversations::{
    ensure_memory_curate_conversation, memory_curate_conversation_key,
};

async fn test_pool() -> Option<PgPool> {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
    let pool = PgPoolOptions::new().connect(&database_url).await.ok()?;
    sqlx::migrate!("./migrations").run(&pool).await.ok()?;
    Some(pool)
}

#[tokio::test]
async fn ensure_memory_curate_conversation_is_idempotent_per_day() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let bear_id = Uuid::new_v4();
    let date = Date::from_calendar_date(2026, time::Month::June, 8).expect("valid date");

    let first = ensure_memory_curate_conversation(&pool, bear_id, Some("curate-agent"), date)
        .await
        .expect("ensure first");
    let second = ensure_memory_curate_conversation(&pool, bear_id, Some("curate-agent"), date)
        .await
        .expect("ensure second");

    assert_eq!(first.id, second.id);
    assert_eq!(first.conversation_key, memory_curate_conversation_key(date));
    assert!(second
        .conversation_id
        .as_deref()
        .is_some_and(|id| id.starts_with("conv-memory-curate-")));
}
