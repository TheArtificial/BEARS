use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use den_http::acp_tokens;
use den_runtime::{
    bearwire_runs,
    bears::{db as bears_db, db::BearParams},
    DenState,
};

use crate::{
    events::{events, EventStreamQuery},
    rpc::{rpc, JsonRpcRequest},
};

fn test_state(pool: sqlx::PgPool) -> DenState {
    let config = std::sync::Arc::new(den_core::config::Config::test_stub());
    DenState::new(
        pool,
        config.clone(),
        std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
        den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
    )
}

async fn create_test_user(pool: &sqlx::PgPool) -> i32 {
    let suffix = Uuid::new_v4().simple().to_string();
    let username = format!("bw{}", &suffix[..16]);
    let email = format!("{username}@example.test");
    let (user_id,): (i32,) = sqlx::query_as(
        r#"
        INSERT INTO users (email, username, display_name, passhash)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(email)
    .bind(&username)
    .bind(format!("BearWire Test {username}"))
    .bind("unused-in-bearwire-tests")
    .fetch_one(pool)
    .await
    .expect("insert test user");
    user_id
}

async fn create_test_bear(pool: &sqlx::PgPool) -> (uuid::Uuid, String) {
    let suffix = Uuid::new_v4().simple().to_string();
    let slug = format!("bearwire-test-{}", &suffix[..12]);
    let bear_id = bears_db::create_bear(
        pool,
        BearParams {
            slug: &slug,
            name: "BearWire Test Bear",
            description: "BearWire integration test bear",
            system_prompt: "test",
            default_model: None,
            tools_enabled: None,
            letta_agent_type: None,
            letta_tool_ids: sqlx::types::Json(Vec::<String>::new()),
            context_profile: None,
        },
    )
    .await
    .expect("create Bear");
    (bear_id, slug)
}

async fn create_token_for_bear(
    pool: &sqlx::PgPool,
    user_id: i32,
    bear_id: uuid::Uuid,
) -> String {
    bears_db::grant_membership(pool, user_id, bear_id, Some(bears_db::BEAR_ROLE_ADMIN))
        .await
        .expect("grant membership");
    acp_tokens::create_for_bear(pool, user_id, bear_id, "BearWire test token")
        .await
        .expect("create token")
        .raw_token
}

fn bearer_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().expect("header value"),
    );
    headers
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_open_persists_event_and_events_replay(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let state = test_state(pool.clone());
    let session_id = format!("session-{}", Uuid::new_v4().simple());

    let response = rpc(
        State(state.clone()),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-open")),
            method: "session.open".to_string(),
            params: json!({
                "bear_slug": bear_slug,
                "session_id": session_id,
                "conversation_id": "conv-bearwire-test",
                "client": "bearwire-test"
            }),
        }),
    )
    .await
    .expect("session.open response")
    .into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["result"]["ok"], true);
    let sequence = value["result"]["event_sequence"].as_i64().unwrap();

    let replay = events(
        State(state),
        bearer_headers(&token),
        Path(session_id.clone()),
        Query(EventStreamQuery {
            bear_slug: value["result"]["session"]["bear_slug"].as_str().unwrap().to_string(),
            after: None,
        }),
    )
    .await
    .expect("events response");
    let replay_body = axum::body::to_bytes(replay.into_body(), usize::MAX)
        .await
        .unwrap();
    let replay_text = std::str::from_utf8(&replay_body).unwrap();
    assert!(replay_text.contains(&format!("id: {sequence}")), "{replay_text}");
    assert!(replay_text.contains("\"type\":\"session.opened\""), "{replay_text}");

    let replay_after = events(
        State(test_state(pool)),
        bearer_headers(&token),
        Path(session_id),
        Query(EventStreamQuery {
            bear_slug: value["result"]["session"]["bear_slug"].as_str().unwrap().to_string(),
            after: Some(sequence),
        }),
    )
    .await
    .expect("events response after cursor");
    let replay_after_body = axum::body::to_bytes(replay_after.into_body(), usize::MAX)
        .await
        .unwrap();
    let replay_after_text = std::str::from_utf8(&replay_after_body).unwrap();
    assert!(!replay_after_text.contains("session.opened"), "{replay_after_text}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn client_result_recording_is_idempotent_and_detects_conflicts(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, _bear_slug) = create_test_bear(&pool).await;
    let run = bearwire_runs::create_run(
        &pool,
        "run-idempotency-test",
        "session-idempotency-test",
        bear_id,
        user_id,
    )
    .await
    .expect("create run");
    assert_eq!(run.state, "accepted");

    let first = bearwire_runs::record_client_result(
        &pool,
        "run-idempotency-test",
        "tool",
        "call-1",
        json!({ "status": "ok", "content": "same" }),
    )
    .await
    .expect("record first result");
    assert!(matches!(
        first,
        bearwire_runs::BearWireClientResultRecord::Inserted { .. }
    ));

    let duplicate = bearwire_runs::record_client_result(
        &pool,
        "run-idempotency-test",
        "tool",
        "call-1",
        json!({ "status": "ok", "content": "same" }),
    )
    .await
    .expect("record duplicate result");
    assert!(matches!(
        duplicate,
        bearwire_runs::BearWireClientResultRecord::DuplicateIdentical { .. }
    ));

    let conflict = bearwire_runs::record_client_result(
        &pool,
        "run-idempotency-test",
        "tool",
        "call-1",
        json!({ "status": "ok", "content": "different" }),
    )
    .await
    .expect("record conflicting result");
    assert!(matches!(
        conflict,
        bearwire_runs::BearWireClientResultRecord::DuplicateConflict { .. }
    ));
}

#[tokio::test]
async fn initialize_returns_bearwire_capabilities() {
    let response = rpc(
        State(test_state(sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap())),
        HeaderMap::new(),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-1")),
            method: "initialize".to_string(),
            params: json!({}),
        }),
    )
    .await
    .expect("initialize ok")
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn planned_v1_methods_are_recognized() {
    let state = test_state(sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap());
    for method in [
        "session.open",
        "session.resume",
        "session.close",
        "session.state",
        "run.start",
        "run.cancel",
        "client.tool.result",
        "client.permission.result",
        "resource.update",
    ] {
        let response = rpc(
            State(state.clone()),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!(method)),
                method: method.to_string(),
                params: json!({ "session_id": "session-test" }),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_ne!(value.pointer("/error/code"), Some(&json!(-32601)), "{method}");
    }
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let response = rpc(
        State(test_state(sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap())),
        HeaderMap::new(),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-unknown")),
            method: "not.real".to_string(),
            params: json!({}),
        }),
    )
    .await
    .expect("rpc ok")
    .into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["error"]["code"], -32601);
}

async fn assert_method_requires_bearer_token(method: &str, params: Value) {
    let response = rpc(
        State(test_state(sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap())),
        HeaderMap::new(),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(method)),
            method: method.to_string(),
            params,
        }),
    )
    .await
    .expect("rpc ok")
    .into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["error"]["code"], -32001);
    assert!(value["error"]["data"]["error"].as_str().unwrap().contains("missing Authorization"));
}

#[tokio::test]
async fn bear_scoped_methods_require_bearer_token() {
    assert_method_requires_bearer_token(
        "session.open",
        json!({ "bear_slug": "meta", "session_id": "session-test" }),
    )
    .await;
    assert_method_requires_bearer_token("session.state", json!({ "bear_slug": "meta" })).await;
    assert_method_requires_bearer_token(
        "run.start",
        json!({ "bear_slug": "meta", "session_id": "session-test", "prompt": "hello" }),
    )
    .await;
    assert_method_requires_bearer_token(
        "run.cancel",
        json!({ "bear_slug": "meta", "session_id": "session-test" }),
    )
    .await;
    assert_method_requires_bearer_token(
        "client.tool.result",
        json!({
            "bear_slug": "meta",
            "session_id": "session-test",
            "run_id": "run-test",
            "tool_call_id": "call-test",
            "status": "ok"
        }),
    )
    .await;
    assert_method_requires_bearer_token(
        "client.permission.result",
        json!({
            "bear_slug": "meta",
            "session_id": "session-test",
            "run_id": "run-test",
            "permission_id": "perm-test",
            "decision": "approved"
        }),
    )
    .await;
    assert_method_requires_bearer_token(
        "resource.update",
        json!({
            "bear_slug": "meta",
            "session_id": "session-test",
            "resource": { "kind": "acp_adapter", "id": "armature-test" }
        }),
    )
    .await;
}
