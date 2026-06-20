use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

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
    acp_sessions,
    bears::{db as bears_db, db::BearParams},
    bearwire_obligations, bearwire_runs, DenState,
};

use crate::{
    events::{events, EventStreamQuery},
    rpc::{rpc, JsonRpcRequest},
};

fn test_state(pool: sqlx::PgPool) -> DenState {
    test_state_with_config(pool, den_core::config::Config::test_stub())
}

fn test_state_with_config(pool: sqlx::PgPool, config: den_core::config::Config) -> DenState {
    let config = std::sync::Arc::new(config);
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
    bears_db::ensure_bear_profile_binding_rows(pool, bear_id)
        .await
        .expect("ensure Bear profile bindings");
    (bear_id, slug)
}

async fn create_token_for_bear(pool: &sqlx::PgPool, user_id: i32, bear_id: uuid::Uuid) -> String {
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

async fn upsert_test_session(
    pool: &sqlx::PgPool,
    user_id: i32,
    bear_id: uuid::Uuid,
    bear_slug: &str,
    session_id: &str,
) {
    acp_sessions::upsert_session(
        pool,
        acp_sessions::UpsertAcpSession {
            user_id,
            bear_id,
            bear_slug: bear_slug.to_string(),
            acp_session_id: session_id.to_string(),
            runtime_session_id: format!("bearwire-test:{bear_id}:{session_id}"),
            conversation_id: format!("den-conv-{}", Uuid::new_v4().simple()),
            resolved_conversation_id: None,
            client: "bearwire-test".to_string(),
            cwd: Some("/workspace".to_string()),
            current_mode: Some("write".to_string()),
        },
    )
    .await
    .expect("upsert BearWire test session");
}

async fn rpc_value(state: DenState, token: &str, method: &str, params: Value) -> Value {
    let response = rpc(
        State(state),
        bearer_headers(token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(format!("req-{}", Uuid::new_v4().simple()))),
            method: method.to_string(),
            params,
        }),
    )
    .await
    .expect("rpc response")
    .into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
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
            bear_slug: value["result"]["session"]["bear_slug"]
                .as_str()
                .unwrap()
                .to_string(),
            after: None,
        }),
    )
    .await
    .expect("events response");
    let replay_body = axum::body::to_bytes(replay.into_body(), usize::MAX)
        .await
        .unwrap();
    let replay_text = std::str::from_utf8(&replay_body).unwrap();
    assert!(
        replay_text.contains(&format!("id: {sequence}")),
        "{replay_text}"
    );
    assert!(
        replay_text.contains("\"type\":\"session.opened\""),
        "{replay_text}"
    );

    let replay_after = events(
        State(test_state(pool)),
        bearer_headers(&token),
        Path(session_id),
        Query(EventStreamQuery {
            bear_slug: value["result"]["session"]["bear_slug"]
                .as_str()
                .unwrap()
                .to_string(),
            after: Some(sequence),
        }),
    )
    .await
    .expect("events response after cursor");
    let replay_after_body = axum::body::to_bytes(replay_after.into_body(), usize::MAX)
        .await
        .unwrap();
    let replay_after_text = std::str::from_utf8(&replay_after_body).unwrap();
    assert!(
        !replay_after_text.contains("session.opened"),
        "{replay_after_text}"
    );
}

fn start_mock_openai_sse_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock LLM server");
    let addr = listener.local_addr().expect("mock LLM local addr");
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let request = read_http_request(&mut stream);
            assert!(
                request.starts_with("POST /chat/completions "),
                "unexpected LLM request: {request}"
            );
            let body = concat!(
                "data: {\"id\":\"chatcmpl-bearwire-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello from bearwire\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"chatcmpl-bearwire-test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write mock LLM response");
        }
    });
    format!("http://{addr}")
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut temp = [0_u8; 1024];
    let mut header_end = None;
    while header_end.is_none() {
        let read = stream.read(&mut temp).expect("read mock LLM request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..read]);
        header_end = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    }

    let Some(header_end) = header_end else {
        return String::from_utf8_lossy(&buffer).into_owned();
    };
    let header_text = String::from_utf8_lossy(&buffer[..header_end + 4]);
    let content_length = header_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let already_read_body = buffer.len().saturating_sub(header_end + 4);
    let remaining = content_length.saturating_sub(already_read_body);
    if remaining > 0 {
        let mut body = vec![0_u8; remaining];
        stream
            .read_exact(&mut body)
            .expect("read mock LLM request body");
        buffer.extend_from_slice(&body);
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

async fn replay_events_text(
    state: DenState,
    token: &str,
    bear_slug: &str,
    session_id: &str,
) -> String {
    let replay = events(
        State(state),
        bearer_headers(token),
        Path(session_id.to_string()),
        Query(EventStreamQuery {
            bear_slug: bear_slug.to_string(),
            after: None,
        }),
    )
    .await
    .expect("events response");
    let replay_body = axum::body::to_bytes(replay.into_body(), usize::MAX)
        .await
        .unwrap();
    std::str::from_utf8(&replay_body).unwrap().to_string()
}

#[sqlx::test(migrations = "../../migrations")]
async fn run_start_persists_message_delta_and_completed_events_for_mock_llm(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let mut config = den_core::config::Config::test_stub();
    config.llm_api_url = start_mock_openai_sse_server();
    config.default_llm_model = "openai/bearwire-test-model".to_string();
    let state = test_state_with_config(pool.clone(), config);
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let conversation_id = format!("conv-{}", Uuid::new_v4().simple());

    let response = rpc(
        State(state.clone()),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-run-start")),
            method: "run.start".to_string(),
            params: json!({
                "bear_slug": bear_slug,
                "session_id": session_id,
                "conversation_id": conversation_id,
                "client": "bearwire-test",
                "prompt": "Say hello."
            }),
        }),
    )
    .await
    .expect("run.start response")
    .into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["result"]["ok"], true, "{value}");
    assert_eq!(value["result"]["accepted"], true, "{value}");

    let mut last_replay = String::new();
    for _ in 0..40 {
        last_replay = replay_events_text(state.clone(), &token, &bear_slug, &session_id).await;
        if last_replay.contains("\"type\":\"message.delta\"")
            && last_replay.contains("\"type\":\"run.completed\"")
        {
            assert!(last_replay.contains("hello from bearwire"), "{last_replay}");
            assert!(
                last_replay.contains("\"type\":\"run.accepted\""),
                "{last_replay}"
            );
            assert!(
                last_replay.contains("\"type\":\"run.started\""),
                "{last_replay}"
            );
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }

    panic!(
        "BearWire run.start did not persist message.delta and run.completed events: {last_replay}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_state_auth_error_reports_specific_token_bear_diagnostics(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let (other_bear_id, other_bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    bears_db::grant_membership(
        &pool,
        user_id,
        other_bear_id,
        Some(bears_db::BEAR_ROLE_ADMIN),
    )
    .await
    .expect("grant membership to other Bear");

    let response = rpc(
        State(test_state(pool)),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-state-diagnostics")),
            method: "session.state".to_string(),
            params: json!({
                "bear_slug": other_bear_slug,
                "limit": 1,
            }),
        }),
    )
    .await
    .expect("session.state response")
    .into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    let error = value["error"]["data"]["error"].as_str().unwrap();
    assert!(error.contains("token_found=true"), "{error}");
    assert!(error.contains("bear_found=true"), "{error}");
    assert!(error.contains("token_bound_to_bear=false"), "{error}");
    assert!(error.contains("token_owner_is_bear_member=true"), "{error}");
    assert!(error.contains("required_scope_present=true"), "{error}");
    assert!(
        error.contains("token is not granted to this Bear"),
        "{error}"
    );
    assert!(
        error.contains(&format!("bear_slug=\"{}\"", other_bear_slug)),
        "{error}"
    );
    assert!(
        !error.contains(&token),
        "diagnostics must not echo raw token"
    );
    assert!(
        !error.contains(&bear_slug),
        "diagnostics should only report requested Bear slug"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_state_auth_error_reports_missing_bear_slug(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, _bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let missing_slug = format!("missing-bear-{}", Uuid::new_v4().simple());

    let response = rpc(
        State(test_state(pool)),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-missing-bear")),
            method: "session.state".to_string(),
            params: json!({
                "bear_slug": missing_slug,
                "limit": 1,
            }),
        }),
    )
    .await
    .expect("session.state response")
    .into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    let error = value["error"]["data"]["error"].as_str().unwrap();
    assert!(error.contains("token_found=true"), "{error}");
    assert!(error.contains("bear_found=false"), "{error}");
    assert!(
        error.contains("bear slug does not exist in this Den database"),
        "{error}"
    );
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

#[sqlx::test(migrations = "../../migrations")]
async fn client_result_methods_reject_wrong_obligation_kind(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    bearwire_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");

    bearwire_obligations::upsert_permission_obligation(
        &pool,
        &run_id,
        &session_id,
        "perm-wrong-tool-route",
        Some("call-wrong-tool-route"),
        json!({ "test": "permission obligation" }),
    )
    .await
    .expect("insert permission obligation");
    let tool_response = rpc_value(
        test_state(pool.clone()),
        &token,
        "client.tool.result",
        json!({
            "bear_slug": bear_slug,
            "session_id": session_id,
            "run_id": run_id,
            "tool_call_id": "call-wrong-tool-route",
            "status": "ok",
            "content": "not accepted by permission obligation"
        }),
    )
    .await;
    let tool_error = tool_response["error"]["data"]["error"].as_str().unwrap();
    assert!(
        tool_error.contains("does not accept client.tool.result"),
        "{tool_response}"
    );

    bearwire_obligations::upsert_tool_call_obligation(
        &pool,
        &run_id,
        &session_id,
        "call-wrong-permission-route",
        Some("perm-wrong-permission-route"),
        json!({ "test": "tool obligation" }),
    )
    .await
    .expect("insert tool obligation");
    let permission_response = rpc_value(
        test_state(pool.clone()),
        &token,
        "client.permission.result",
        json!({
            "bear_slug": bear_slug,
            "session_id": session_id,
            "run_id": run_id,
            "permission_id": "perm-wrong-permission-route",
            "decision": "approved"
        }),
    )
    .await;
    let permission_error = permission_response["error"]["data"]["error"]
        .as_str()
        .unwrap();
    assert!(
        permission_error.contains("does not accept client.permission.result"),
        "{permission_response}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn tool_result_uses_persisted_obligation_after_fresh_state_and_stays_idempotent(
    pool: sqlx::PgPool,
) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let tool_call_id = format!("call_{}", Uuid::new_v4().simple());
    upsert_test_session(&pool, user_id, bear_id, &bear_slug, &session_id).await;
    bearwire_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");
    bearwire_runs::transition_run(
        &pool,
        &run_id,
        bearwire_runs::BearWireRunState::Running,
        None,
    )
    .await
    .expect("transition run to running");
    bearwire_obligations::upsert_tool_call_obligation(
        &pool,
        &run_id,
        &session_id,
        &tool_call_id,
        None,
        json!({ "test": "fresh-state persisted obligation" }),
    )
    .await
    .expect("insert tool obligation");

    let params = json!({
        "bear_slug": bear_slug,
        "session_id": session_id,
        "run_id": run_id,
        "tool_call_id": tool_call_id,
        "status": "ok",
        "content": "persisted tool result"
    });
    let response = rpc_value(
        test_state(pool.clone()),
        &token,
        "client.tool.result",
        params.clone(),
    )
    .await;
    assert_eq!(response["result"]["ok"], true, "{response}");
    assert_eq!(response["result"]["duplicate"], false, "{response}");
    let obligation = bearwire_obligations::get_tool_call_obligation(&pool, &run_id, &tool_call_id)
        .await
        .expect("load obligation")
        .expect("obligation exists");
    assert_eq!(obligation.state, "continued");

    let duplicate = rpc_value(
        test_state(pool.clone()),
        &token,
        "client.tool.result",
        params.clone(),
    )
    .await;
    assert_eq!(duplicate["result"]["ok"], true, "{duplicate}");
    assert_eq!(duplicate["result"]["duplicate"], true, "{duplicate}");
    assert_eq!(
        duplicate["result"]["obligation_state"], "continued",
        "{duplicate}"
    );

    let conflict = rpc_value(
        test_state(pool.clone()),
        &token,
        "client.tool.result",
        json!({
            "bear_slug": params["bear_slug"].clone(),
            "session_id": params["session_id"].clone(),
            "run_id": params["run_id"].clone(),
            "tool_call_id": params["tool_call_id"].clone(),
            "status": "ok",
            "content": "different result"
        }),
    )
    .await;
    let error = conflict["error"]["data"]["error"].as_str().unwrap();
    assert!(
        error.contains("conflicting duplicate tool result"),
        "{conflict}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn same_session_rejects_second_active_run(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_a = format!("run_{}", Uuid::new_v4().simple());
    let run_b = format!("run_{}", Uuid::new_v4().simple());
    upsert_test_session(&pool, user_id, bear_id, &bear_slug, &session_id).await;
    bearwire_runs::create_run(&pool, &run_a, &session_id, bear_id, user_id)
        .await
        .expect("create first active run");

    let err = bearwire_runs::create_run(&pool, &run_b, &session_id, bear_id, user_id)
        .await
        .expect_err("second active run in one ACP session should be rejected");
    assert!(
        err.to_string()
            .contains("idx_bearwire_runs_one_active_per_session"),
        "unexpected error: {err}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn cross_session_tool_call_id_collision_is_isolated_by_run_and_session(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let session_a = format!("session-a-{}", Uuid::new_v4().simple());
    let session_b = format!("session-b-{}", Uuid::new_v4().simple());
    let run_a = format!("run_{}", Uuid::new_v4().simple());
    let run_b = format!("run_{}", Uuid::new_v4().simple());
    let tool_call_id = "call-collision";
    upsert_test_session(&pool, user_id, bear_id, &bear_slug, &session_a).await;
    upsert_test_session(&pool, user_id, bear_id, &bear_slug, &session_b).await;
    bearwire_runs::create_run(&pool, &run_a, &session_a, bear_id, user_id)
        .await
        .expect("create run a");
    bearwire_runs::create_run(&pool, &run_b, &session_b, bear_id, user_id)
        .await
        .expect("create run b");
    bearwire_obligations::upsert_tool_call_obligation(
        &pool,
        &run_a,
        &session_a,
        tool_call_id,
        None,
        json!({ "session": "a" }),
    )
    .await
    .expect("insert session a obligation");
    bearwire_obligations::upsert_tool_call_obligation(
        &pool,
        &run_b,
        &session_b,
        tool_call_id,
        None,
        json!({ "session": "b" }),
    )
    .await
    .expect("insert session b obligation");

    let wrong_session = rpc_value(
        test_state(pool.clone()),
        &token,
        "client.tool.result",
        json!({
            "bear_slug": bear_slug,
            "session_id": session_b,
            "run_id": run_a,
            "tool_call_id": tool_call_id,
            "status": "ok",
            "content": "wrong session"
        }),
    )
    .await;
    let error = wrong_session["error"]["data"]["error"].as_str().unwrap();
    assert!(
        error.contains("run does not belong to authenticated Bear/session"),
        "{wrong_session}"
    );

    let response = rpc_value(
        test_state(pool.clone()),
        &token,
        "client.tool.result",
        json!({
            "bear_slug": bear_slug,
            "session_id": session_a,
            "run_id": run_a,
            "tool_call_id": tool_call_id,
            "status": "ok",
            "content": "correct session"
        }),
    )
    .await;
    assert_eq!(response["result"]["ok"], true, "{response}");

    let obligation_a = bearwire_obligations::get_tool_call_obligation(&pool, &run_a, tool_call_id)
        .await
        .expect("load session a obligation")
        .expect("session a obligation exists");
    let obligation_b = bearwire_obligations::get_tool_call_obligation(&pool, &run_b, tool_call_id)
        .await
        .expect("load session b obligation")
        .expect("session b obligation exists");
    assert_eq!(obligation_a.state, "continued");
    assert_eq!(obligation_b.state, "waiting_for_client");
}

#[sqlx::test(migrations = "../../migrations")]
async fn run_cancel_settles_outstanding_obligations(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    upsert_test_session(&pool, user_id, bear_id, &bear_slug, &session_id).await;
    bearwire_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");
    bearwire_obligations::upsert_tool_call_obligation(
        &pool,
        &run_id,
        &session_id,
        "call-cancelled",
        Some("perm-cancelled"),
        json!({ "test": "tool obligation" }),
    )
    .await
    .expect("insert tool obligation");
    bearwire_obligations::upsert_permission_obligation(
        &pool,
        &run_id,
        &session_id,
        "perm-cancelled",
        Some("call-cancelled"),
        json!({ "test": "permission obligation" }),
    )
    .await
    .expect("insert permission obligation");

    let response = rpc_value(
        test_state(pool.clone()),
        &token,
        "run.cancel",
        json!({
            "bear_slug": bear_slug,
            "session_id": session_id,
        }),
    )
    .await;
    assert_eq!(response["result"]["ok"], true, "{response}");
    assert_eq!(response["result"]["cancelled"], true, "{response}");
    assert_eq!(response["result"]["run_id"], run_id, "{response}");

    let tool = bearwire_obligations::get_tool_call_obligation(&pool, &run_id, "call-cancelled")
        .await
        .expect("load tool obligation")
        .expect("tool obligation exists");
    let permission =
        bearwire_obligations::get_permission_obligation(&pool, &run_id, "perm-cancelled")
            .await
            .expect("load permission obligation")
            .expect("permission obligation exists");
    assert_eq!(tool.state, "cancelled");
    assert_eq!(permission.state, "cancelled");
}

#[tokio::test]
async fn initialize_returns_bearwire_capabilities() {
    let response = rpc(
        State(test_state(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
        )),
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
    let state = test_state(
        sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
    );
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
        assert_ne!(
            value.pointer("/error/code"),
            Some(&json!(-32601)),
            "{method}"
        );
    }
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let response = rpc(
        State(test_state(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
        )),
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
        State(test_state(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
        )),
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
    assert!(value["error"]["data"]["error"]
        .as_str()
        .unwrap()
        .contains("missing Authorization"));
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
