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
use sqlx::Row;
use uuid::Uuid;

use den_http::armature_tokens;
use den_protocol::{
    ContextBudgetComponentReport, ContextBudgetEstimatePrecision, ContextBudgetReport,
    RoleRuntimeBinding, RuntimeConversationBackend, RuntimeConversationRef, RuntimeSemanticEvent,
    RuntimeStreamEvent,
};
use den_runtime::{native_runtime::NativeRuntimeConversationBackend, turn_obligations, turn_runs};
use den_service::{
    bears::{db as bears_db, db::BearParams},
    client_sessions,
    conversation::persistence::{
        append_message, ensure_conversation_for_external_id, update_latest_context_budget,
    },
    conversation_message_types::{
        ConversationMessageRole, ConversationMessageType, ConversationMessageVisibility,
        ConversationMessageWrite,
    },
    DenState,
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
    let state = DenState::new(
        pool,
        config.clone(),
        std::sync::Arc::new(den_service::bifrost::BifrostClient::new(config.as_ref())),
        den_memory::MemoryStoreManager::new(config.as_ref()),
    );
    let snapshot = den_service::bifrost::BifrostCatalogSnapshot::from_available_models(vec![
        den_service::bifrost::BifrostModelMetadata {
            handle: den_llm::normalize_llm_model_handle(&config.default_llm_model),
            provider: "openai".to_string(),
            model: config.default_llm_model.trim().to_string(),
            display_name: Some("BearWire test model".to_string()),
            context_window: 128_000,
            max_output_tokens: Some(4096),
            enabled: true,
            supports_tools: Some(true),
            supports_responses_api: Some(false),
            supports_vision: Some(false),
        },
    ]);
    *state.bifrost_catalog.write().expect("catalog lock") = snapshot;
    state
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

async fn seed_test_bifrost_virtual_key(
    pool: &sqlx::PgPool,
    bear_id: uuid::Uuid,
    config: &den_core::config::Config,
) {
    bears_db::set_bear_bifrost_virtual_key(
        pool,
        bear_id,
        Some("vk-test"),
        Some("BearWire test virtual key"),
        Some("sk-bf-bearwire-test"),
        &config.den_secret_encryption_key,
    )
    .await
    .expect("seed test Bifrost virtual key");
}

async fn create_token_for_bear(pool: &sqlx::PgPool, user_id: i32, bear_id: uuid::Uuid) -> String {
    bears_db::grant_membership(pool, user_id, bear_id, Some(bears_db::BEAR_ROLE_ADMIN))
        .await
        .expect("grant membership");
    armature_tokens::create_for_bear(pool, user_id, bear_id, "BearWire test token")
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
    client_sessions::upsert_session(
        pool,
        client_sessions::UpsertClientSession {
            user_id,
            bear_id,
            bear_slug: bear_slug.to_string(),
            client_session_id: session_id.to_string(),
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
    start_mock_openai_sse_server_asserting_body(Vec::new())
}

#[derive(Debug, Clone)]
struct MockLlmRequestAssertion {
    required_body_substrings: Vec<String>,
    exact_body_counts: Vec<(String, usize)>,
}

impl MockLlmRequestAssertion {
    fn requiring(required_body_substrings: Vec<String>) -> Self {
        Self {
            required_body_substrings,
            exact_body_counts: Vec::new(),
        }
    }
}

fn start_mock_openai_sse_server_asserting_body(required_body_substrings: Vec<String>) -> String {
    start_mock_openai_sse_server_asserting_requests(vec![MockLlmRequestAssertion::requiring(
        required_body_substrings,
    )])
}

fn start_mock_openai_sse_server_asserting_requests(
    request_assertions: Vec<MockLlmRequestAssertion>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock LLM server");
    let addr = listener.local_addr().expect("mock LLM local addr");
    thread::spawn(move || {
        for assertion in request_assertions {
            let (request, mut stream) = loop {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let request = read_http_request(&mut stream);
                if request.starts_with("GET /models") {
                    let body = r#"{"data":[{"id":"gpt-4.1","name":"GPT-4.1","owned_by":"openai","context_length":1047576,"max_output_tokens":32768,"supported_parameters":["tools"],"supported_methods":["chat_completion"]},{"id":"openai/bearwire-test-model","name":"BearWire test model","owned_by":"openai","context_length":128000,"max_output_tokens":4096,"supported_parameters":["tools"],"supported_methods":["chat_completion"]}]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write mock models response");
                    continue;
                }
                break (request, stream);
            };
            assert!(
                request.starts_with("POST /chat/completions "),
                "unexpected LLM request: {request}"
            );
            for needle in &assertion.required_body_substrings {
                assert!(
                    request.contains(needle),
                    "LLM request body missing expected substring {needle:?}: {request}"
                );
            }
            for (needle, expected_count) in &assertion.exact_body_counts {
                let actual_count = request.matches(needle).count();
                assert_eq!(
                    actual_count, *expected_count,
                    "LLM request body had unexpected count for {needle:?}: {request}"
                );
            }
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
    config.den_secret_encryption_key = "bearwire-test-secret-key".to_string();
    config.llm_api_url = start_mock_openai_sse_server();
    config.default_llm_model = "openai/bearwire-test-model".to_string();
    seed_test_bifrost_virtual_key(&pool, bear_id, &config).await;
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
async fn run_start_persists_user_prompt_for_future_history(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let mut config = den_core::config::Config::test_stub();
    config.den_secret_encryption_key = "bearwire-test-secret-key".to_string();
    config.llm_api_url = start_mock_openai_sse_server();
    config.default_llm_model = "openai/bearwire-test-model".to_string();
    seed_test_bifrost_virtual_key(&pool, bear_id, &config).await;
    let state = test_state_with_config(pool.clone(), config);
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let prompt = "Remember this first prompt for future turns";

    let response = rpc(
        State(state.clone()),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-persist-user-prompt")),
            method: "run.start".to_string(),
            params: json!({
                "bear_slug": bear_slug,
                "session_id": session_id,
                "conversation_id": format!("new-acp-zed-{}", Uuid::new_v4().simple()),
                "client": "zed",
                "prompt": prompt
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
    let session =
        client_sessions::find_for_user_bear_session(&pool, user_id, &bear_slug, &session_id)
            .await
            .expect("load session")
            .expect("session exists");
    let resolved = session
        .resolved_conversation_id
        .as_deref()
        .expect("run.start should resolve conversation");
    let (count,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM conversation_messages
        WHERE conversation_id = (
            SELECT id FROM conversations
            WHERE bear_id = $1 AND external_conversation_id = $2
            LIMIT 1
        )
        AND message_type = 'user'
        AND role = 'user'
        AND content_text = $3
        "#,
    )
    .bind(bear_id)
    .bind(resolved)
    .bind(prompt)
    .fetch_one(&pool)
    .await
    .expect("count persisted user prompt");
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn run_start_persists_wrapped_host_context_as_structured_metadata(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let mut config = den_core::config::Config::test_stub();
    config.den_secret_encryption_key = "bearwire-test-secret-key".to_string();
    config.llm_api_url = start_mock_openai_sse_server();
    config.default_llm_model = "openai/bearwire-test-model".to_string();
    seed_test_bifrost_virtual_key(&pool, bear_id, &config).await;
    let state = test_state_with_config(pool.clone(), config);
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let prompt = "Please inspect the library entrypoint.";
    let prompt_context = json!({
        "format": "acp_prompt_context.v1",
        "host_context": {
            "kind": "referenced_resources",
            "delivery": "reference_only",
            "persistence": "not_human_message",
            "resources": [
                {
                    "label": "src/lib.rs",
                    "uri": "file:///workspace/src/lib.rs",
                    "name": "src/lib.rs",
                    "mime_type": "text/rust",
                    "embedded_text_bytes": 128
                }
            ]
        }
    });

    let response = rpc(
        State(state.clone()),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-persist-host-context")),
            method: "run.start".to_string(),
            params: json!({
                "bear_slug": bear_slug,
                "session_id": session_id,
                "conversation_id": format!("new-acp-zed-{}", Uuid::new_v4().simple()),
                "client": "zed",
                "prompt": prompt,
                "prompt_context": prompt_context
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

    let session =
        client_sessions::find_for_user_bear_session(&pool, user_id, &bear_slug, &session_id)
            .await
            .expect("load session")
            .expect("session exists");
    let resolved = session
        .resolved_conversation_id
        .as_deref()
        .expect("run.start should resolve conversation");

    let row = sqlx::query(
        r#"
        SELECT content_text, content_json
        FROM conversation_messages
        WHERE conversation_id = (
            SELECT id FROM conversations
            WHERE bear_id = $1 AND external_conversation_id = $2
            LIMIT 1
        )
        AND message_type = 'user'
        AND role = 'user'
        ORDER BY sequence_no DESC
        LIMIT 1
        "#,
    )
    .bind(bear_id)
    .bind(resolved)
    .fetch_one(&pool)
    .await
    .expect("load persisted user prompt row");

    let persisted_text: String = row.try_get("content_text").expect("decode content_text");
    let persisted_json: Value = row.try_get("content_json").expect("decode content_json");
    assert_eq!(persisted_text, "Please inspect the library entrypoint.");
    assert_eq!(
        persisted_json["prompt_context"]["format"],
        "acp_prompt_context.v1"
    );
    assert_eq!(
        persisted_json["host_context"]["kind"],
        "referenced_resources"
    );
    assert_eq!(persisted_json["host_context"]["delivery"], "reference_only");
    assert_eq!(
        persisted_json["host_context"]["persistence"],
        "not_human_message"
    );
    assert_eq!(
        persisted_json["host_context"]["resources"][0]["uri"],
        "file:///workspace/src/lib.rs"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn run_start_second_turn_replays_first_user_and_assistant_once(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let first_prompt = "First prompt: remember the blue teapot";
    let second_prompt = "What was my first prompt?";
    let mut second = MockLlmRequestAssertion::requiring(vec![
        first_prompt.to_string(),
        "hello from bearwire".to_string(),
        second_prompt.to_string(),
    ]);
    second
        .exact_body_counts
        .push((second_prompt.to_string(), 1));

    let mut config = den_core::config::Config::test_stub();
    config.den_secret_encryption_key = "bearwire-test-secret-key".to_string();
    config.llm_api_url = start_mock_openai_sse_server_asserting_requests(vec![
        MockLlmRequestAssertion {
            required_body_substrings: vec![first_prompt.to_string()],
            exact_body_counts: vec![(first_prompt.to_string(), 1)],
        },
        second,
    ]);
    config.default_llm_model = "openai/bearwire-test-model".to_string();
    seed_test_bifrost_virtual_key(&pool, bear_id, &config).await;
    let state = test_state_with_config(pool.clone(), config);

    let first_response = rpc(
        State(state.clone()),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-first-history-turn")),
            method: "run.start".to_string(),
            params: json!({
                "bear_slug": bear_slug,
                "session_id": session_id,
                "conversation_id": format!("new-acp-zed-{}", Uuid::new_v4().simple()),
                "client": "zed",
                "prompt": first_prompt
            }),
        }),
    )
    .await
    .expect("first run.start response")
    .into_response();
    let body = axum::body::to_bytes(first_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let first_value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(first_value["result"]["ok"], true, "{first_value}");
    let first_run_id = first_value["result"]["run_id"]
        .as_str()
        .expect("first run_id")
        .to_string();

    let session =
        client_sessions::find_for_user_bear_session(&pool, user_id, &bear_slug, &session_id)
            .await
            .expect("load session")
            .expect("session exists");
    let resolved = session
        .resolved_conversation_id
        .as_deref()
        .expect("first run.start should resolve conversation")
        .to_string();

    let mut first_turn_ready = false;
    for _ in 0..50 {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM turn_runs WHERE run_id = $1 LIMIT 1")
                .bind(&first_run_id)
                .fetch_optional(&pool)
                .await
                .expect("load first run state");
        let assistant_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)::bigint
            FROM conversation_messages
            WHERE conversation_id = (
                SELECT id FROM conversations
                WHERE bear_id = $1 AND external_conversation_id = $2
                LIMIT 1
            )
            AND message_type = 'assistant'
            AND content_text = 'hello from bearwire'
            "#,
        )
        .bind(bear_id)
        .bind(&resolved)
        .fetch_one(&pool)
        .await
        .expect("count first assistant message");
        if state.as_deref() == Some("completed") && assistant_count == 1 {
            first_turn_ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        first_turn_ready,
        "first turn did not complete and persist assistant output before second turn"
    );

    let second_response = rpc(
        State(state.clone()),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-second-history-turn")),
            method: "run.start".to_string(),
            params: json!({
                "bear_slug": bear_slug,
                "session_id": session_id,
                "client": "zed",
                "prompt": second_prompt
            }),
        }),
    )
    .await
    .expect("second run.start response")
    .into_response();
    let body = axum::body::to_bytes(second_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let second_value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(second_value["result"]["ok"], true, "{second_value}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn native_history_loader_replays_canonical_user_and_assistant_rows(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, _) = create_test_bear(&pool).await;
    let conversation_id = format!("den-conv-{}", Uuid::new_v4().simple());
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let canonical = ensure_conversation_for_external_id(
        &pool,
        bear_id,
        Some(user_id),
        &conversation_id,
        Some(&session_id),
        None,
    )
    .await
    .expect("ensure canonical conversation");
    append_message(
        &pool,
        canonical.id,
        &ConversationMessageWrite {
            message_type: ConversationMessageType::User,
            role: Some(ConversationMessageRole::User),
            visibility: ConversationMessageVisibility::Default,
            content_text: "first user prompt".to_string(),
            content_json: json!({}),
            provider_message_id: Some("prior-user".to_string()),
            source_event_id: None,
            created_at: None,
        },
    )
    .await
    .expect("append user message");
    append_message(
        &pool,
        canonical.id,
        &ConversationMessageWrite {
            message_type: ConversationMessageType::Assistant,
            role: Some(ConversationMessageRole::Assistant),
            visibility: ConversationMessageVisibility::Default,
            content_text: "first assistant reply".to_string(),
            content_json: json!({}),
            provider_message_id: Some("prior-assistant".to_string()),
            source_event_id: None,
            created_at: None,
        },
    )
    .await
    .expect("append assistant message");

    let backend = NativeRuntimeConversationBackend::with_pool(pool.clone());
    let binding = RoleRuntimeBinding {
        binding_id: format!("den-native:{bear_id}:pair"),
        compatibility_backend: Some("native".to_string()),
    };
    let history = backend
        .load_history(
            &binding,
            &RuntimeConversationRef {
                id: conversation_id,
            },
        )
        .await
        .expect("load native history");

    assert_eq!(history.records.len(), 2);
    assert_eq!(history.records[0].role, "user");
    assert_eq!(history.records[0].content, "first user prompt");
    assert_eq!(history.records[1].role, "assistant");
    assert_eq!(history.records[1].content, "first assistant reply");
}

#[sqlx::test(migrations = "../../migrations")]
async fn run_start_uses_resolved_conversation_history_for_existing_session(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let pending_conversation_id = format!("new-acp-zed-{}", Uuid::new_v4().simple());
    let resolved_conversation_id = format!("den-conv-{}", Uuid::new_v4().simple());
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    client_sessions::upsert_session(
        &pool,
        client_sessions::UpsertClientSession {
            user_id,
            bear_id,
            bear_slug: bear_slug.clone(),
            client_session_id: session_id.clone(),
            runtime_session_id: format!("bearwire:{bear_id}:{session_id}"),
            conversation_id: pending_conversation_id.clone(),
            resolved_conversation_id: Some(resolved_conversation_id.clone()),
            client: "zed".to_string(),
            cwd: Some("/workspace".to_string()),
            current_mode: Some("write".to_string()),
        },
    )
    .await
    .expect("upsert resolved BearWire session");
    let canonical = ensure_conversation_for_external_id(
        &pool,
        bear_id,
        Some(user_id),
        &resolved_conversation_id,
        Some(&session_id),
        None,
    )
    .await
    .expect("ensure resolved conversation");
    append_message(
        &pool,
        canonical.id,
        &ConversationMessageWrite {
            message_type: ConversationMessageType::User,
            role: Some(ConversationMessageRole::User),
            visibility: ConversationMessageVisibility::Default,
            content_text: "Earlier user asked about cached history".to_string(),
            content_json: json!({}),
            provider_message_id: Some("prior-user".to_string()),
            source_event_id: None,
            created_at: None,
        },
    )
    .await
    .expect("append prior user message");
    append_message(
        &pool,
        canonical.id,
        &ConversationMessageWrite {
            message_type: ConversationMessageType::Assistant,
            role: Some(ConversationMessageRole::Assistant),
            visibility: ConversationMessageVisibility::Default,
            content_text: "Earlier assistant reply from persisted history".to_string(),
            content_json: json!({}),
            provider_message_id: Some("prior-assistant".to_string()),
            source_event_id: None,
            created_at: None,
        },
    )
    .await
    .expect("append prior assistant message");

    let mut config = den_core::config::Config::test_stub();
    config.den_secret_encryption_key = "bearwire-test-secret-key".to_string();
    config.llm_api_url = start_mock_openai_sse_server_asserting_body(vec![
        "Earlier user asked about cached history".to_string(),
        "Earlier assistant reply from persisted history".to_string(),
        "Current turn should see history".to_string(),
    ]);
    config.default_llm_model = "openai/bearwire-test-model".to_string();
    seed_test_bifrost_virtual_key(&pool, bear_id, &config).await;
    let state = test_state_with_config(pool.clone(), config);
    let response = rpc(
        State(state.clone()),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-history-run-start")),
            method: "run.start".to_string(),
            params: json!({
                "bear_slug": bear_slug,
                "session_id": session_id,
                "client": "zed",
                "prompt": "Current turn should see history"
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
async fn session_state_includes_latest_context_budget_for_resolved_conversation(
    pool: sqlx::PgPool,
) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let pending_conversation_id = format!("pending-{}", Uuid::new_v4().simple());
    let resolved_conversation_id = format!("resolved-{}", Uuid::new_v4().simple());
    client_sessions::upsert_session(
        &pool,
        client_sessions::UpsertClientSession {
            user_id,
            bear_id,
            bear_slug: bear_slug.clone(),
            client_session_id: session_id.clone(),
            runtime_session_id: format!("bearwire:{bear_id}:{session_id}"),
            conversation_id: pending_conversation_id,
            resolved_conversation_id: Some(resolved_conversation_id.clone()),
            client: "zed".to_string(),
            cwd: Some("/workspace".to_string()),
            current_mode: Some("write".to_string()),
        },
    )
    .await
    .expect("upsert session");

    let report = ContextBudgetReport {
        model: "openai/test-model".to_string(),
        context_window: Some(128_000),
        max_output_tokens: Some(4_096),
        reserved_output_tokens: 4_096,
        estimated_input_tokens: 12_345,
        estimated_total_tokens: 16_441,
        estimate_precision: ContextBudgetEstimatePrecision::Approximate,
        near_budget: false,
        over_budget: false,
        components: vec![ContextBudgetComponentReport {
            key: "history".to_string(),
            label: "Conversation history".to_string(),
            estimated_tokens: 12_000,
            estimated_characters: 48_000,
        }],
    };
    update_latest_context_budget(
        &pool,
        bear_id,
        &resolved_conversation_id,
        Some(&session_id),
        &report,
    )
    .await
    .expect("persist latest context budget");

    let response = rpc(
        State(test_state(pool)),
        bearer_headers(&token),
        Json(JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!("req-context-budget")),
            method: "session.state".to_string(),
            params: json!({
                "bear_slug": bear_slug,
                "session_id": session_id,
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

    assert_eq!(
        value.pointer("/result/session/context_budget").cloned(),
        Some(serde_json::to_value(report).unwrap())
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn client_result_recording_is_idempotent_and_detects_conflicts(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, _bear_slug) = create_test_bear(&pool).await;
    let run = turn_runs::create_run(
        &pool,
        "run-idempotency-test",
        "session-idempotency-test",
        bear_id,
        user_id,
    )
    .await
    .expect("create run");
    assert_eq!(run.state, "accepted");

    let first = turn_runs::record_client_result(
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
        turn_runs::TurnObligationResultRecord::Inserted { .. }
    ));

    let duplicate = turn_runs::record_client_result(
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
        turn_runs::TurnObligationResultRecord::DuplicateIdentical { .. }
    ));

    let conflict = turn_runs::record_client_result(
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
        turn_runs::TurnObligationResultRecord::DuplicateConflict { .. }
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn client_result_methods_reject_wrong_obligation_kind(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");

    turn_obligations::upsert_permission_decision_obligation(
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

    turn_obligations::upsert_tool_result_obligation(
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
async fn tool_result_without_live_native_session_is_not_accepted_for_continuation(
    pool: sqlx::PgPool,
) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let token = create_token_for_bear(&pool, user_id, bear_id).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let tool_call_id = format!("call_{}", Uuid::new_v4().simple());
    upsert_test_session(&pool, user_id, bear_id, &bear_slug, &session_id).await;
    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");
    turn_runs::transition_run(&pool, &run_id, turn_runs::TurnRunState::Running, None)
        .await
        .expect("transition run to running");
    turn_obligations::upsert_tool_result_obligation(
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
    assert_eq!(response["result"]["ok"], false, "{response}");
    assert_eq!(
        response["result"]["status"], "continuation_unavailable",
        "{response}"
    );
    assert_eq!(
        response["result"]["reason"], "native_agent_loop_session_not_found",
        "{response}"
    );
    assert_eq!(
        response["result"]["diagnostic"]["run_id"], run_id,
        "{response}"
    );
    let obligation = turn_obligations::get_tool_call_obligation(&pool, &run_id, &tool_call_id)
        .await
        .expect("load obligation")
        .expect("obligation exists");
    assert_eq!(obligation.state, "waiting_for_client");
    let recorded = turn_runs::existing_client_result_for_payload(
        &pool,
        &run_id,
        "tool",
        &tool_call_id,
        &json!({
            "tool_call_id": tool_call_id,
            "status": "ok",
            "content": "persisted tool result",
            "structured_content": Value::Null,
            "error": Value::Null,
        }),
    )
    .await
    .expect("query existing result");
    assert!(
        recorded.is_none(),
        "result must not be recorded when continuation is unavailable"
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
    turn_runs::create_run(&pool, &run_a, &session_id, bear_id, user_id)
        .await
        .expect("create first active run");

    let err = turn_runs::create_run(&pool, &run_b, &session_id, bear_id, user_id)
        .await
        .expect_err("second active run in one ACP session should be rejected");
    assert!(
        err.to_string()
            .contains("idx_turn_runs_one_active_per_session"),
        "unexpected error: {err}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn superseding_active_run_allows_new_run_for_session(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_a = format!("run_{}", Uuid::new_v4().simple());
    let run_b = format!("run_{}", Uuid::new_v4().simple());
    upsert_test_session(&pool, user_id, bear_id, &bear_slug, &session_id).await;
    turn_runs::create_run(&pool, &run_a, &session_id, bear_id, user_id)
        .await
        .expect("create first active run");

    let superseded = turn_runs::supersede_active_run_for_session(
        &pool,
        &session_id,
        bear_id,
        user_id,
        "superseded_by_new_run",
    )
    .await
    .expect("supersede active run")
    .expect("active run should be superseded");
    assert_eq!(superseded.run_id, run_a);
    assert_eq!(superseded.state, "failed");

    let created = turn_runs::create_run(&pool, &run_b, &session_id, bear_id, user_id)
        .await
        .expect("create replacement active run");
    assert_eq!(created.run_id, run_b);
}

#[sqlx::test(migrations = "../../migrations")]
async fn approval_required_tool_request_creates_permission_obligation(pool: sqlx::PgPool) {
    let user_id = create_test_user(&pool).await;
    let (bear_id, bear_slug) = create_test_bear(&pool).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let tool_call_id = "call-needs-permission";
    let permission_id = "perm-needs-permission";
    upsert_test_session(&pool, user_id, bear_id, &bear_slug, &session_id).await;
    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create active run");

    crate::methods::run::persist_runtime_event_as_bearwire(
        &pool,
        &session_id,
        &run_id,
        bear_id,
        user_id,
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "fs_list_directory".to_string(),
            title: None,
            kind: Some("read".to_string()),
            arguments: json!({ "path": "/workspace" }),
            approval_request_id: Some(permission_id.to_string()),
            approval_required: true,
            approval_reason: Some("needs approval".to_string()),
            run_id: Some(run_id.clone()),
        }),
        Uuid::new_v4(),
        None,
    )
    .await;

    let obligation = turn_obligations::get_permission_obligation(&pool, &run_id, permission_id)
        .await
        .expect("load permission obligation")
        .expect("permission obligation exists");
    assert_eq!(obligation.expected_responder_action, "permission_decision");
    assert_eq!(obligation.tool_call_id.as_deref(), Some(tool_call_id));
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
    turn_runs::create_run(&pool, &run_a, &session_a, bear_id, user_id)
        .await
        .expect("create run a");
    turn_runs::create_run(&pool, &run_b, &session_b, bear_id, user_id)
        .await
        .expect("create run b");
    turn_obligations::upsert_tool_result_obligation(
        &pool,
        &run_a,
        &session_a,
        tool_call_id,
        None,
        json!({ "session": "a" }),
    )
    .await
    .expect("insert session a obligation");
    turn_obligations::upsert_tool_result_obligation(
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
    assert_eq!(response["result"]["ok"], false, "{response}");
    assert_eq!(
        response["result"]["status"], "continuation_unavailable",
        "{response}"
    );

    let obligation_a = turn_obligations::get_tool_call_obligation(&pool, &run_a, tool_call_id)
        .await
        .expect("load session a obligation")
        .expect("session a obligation exists");
    let obligation_b = turn_obligations::get_tool_call_obligation(&pool, &run_b, tool_call_id)
        .await
        .expect("load session b obligation")
        .expect("session b obligation exists");
    assert_eq!(obligation_a.state, "waiting_for_client");
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
    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");
    turn_obligations::upsert_tool_result_obligation(
        &pool,
        &run_id,
        &session_id,
        "call-cancelled",
        Some("perm-cancelled"),
        json!({ "test": "tool obligation" }),
    )
    .await
    .expect("insert tool obligation");
    turn_obligations::upsert_permission_decision_obligation(
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

    let tool = turn_obligations::get_tool_call_obligation(&pool, &run_id, "call-cancelled")
        .await
        .expect("load tool obligation")
        .expect("tool obligation exists");
    let permission = turn_obligations::get_permission_obligation(&pool, &run_id, "perm-cancelled")
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
