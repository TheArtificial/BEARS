use super::*;

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

#[derive(Debug, Clone)]
struct RequestRecord {
    path: String,
    authorization: Option<String>,
    cookie: Option<String>,
    x_api_key: Option<String>,
    x_bf_vk: Option<String>,
}

#[derive(Clone, Copy)]
enum LoginMode {
    BearerToken,
    CookieSession,
    AuthDisabled,
}

fn test_config(management_url: String) -> Config {
    let mut config = Config::test_stub();
    config.bifrost_management_url = management_url;
    config.bifrost_admin_username = "admin".to_string();
    config.bifrost_admin_password = "password123".to_string();
    config
}

fn spawn_bifrost_management_mock(mode: LoginMode) -> (String, Arc<Mutex<Vec<RequestRecord>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock bifrost server");
    let addr = listener.local_addr().expect("mock server addr");
    let records = Arc::new(Mutex::new(Vec::new()));
    let records_for_thread = Arc::clone(&records);
    let expected_requests = match mode {
        LoginMode::AuthDisabled => 1,
        LoginMode::BearerToken | LoginMode::CookieSession => 2,
    };

    thread::spawn(move || {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().expect("accept mock request");
            let request = read_request(&mut stream);
            let record = request_record(&request);
            let path = record.path.clone();
            records_for_thread
                .lock()
                .expect("records mutex")
                .push(record);

            match (mode, path.as_str()) {
                (LoginMode::BearerToken, "/api/session/login") => {
                    write_response(&mut stream, 200, &[], r#"{"token":"bearer-token-123"}"#)
                }
                (LoginMode::CookieSession, "/api/session/login") => write_response(
                    &mut stream,
                    200,
                    &[(
                        "Set-Cookie",
                        "bifrost_session=session-123; Path=/; HttpOnly",
                    )],
                    r#"{"message":"Login successful"}"#,
                ),
                (LoginMode::AuthDisabled, "/api/session/login") => write_response(
                    &mut stream,
                    403,
                    &[],
                    r#"{"is_bifrost_error":false,"status_code":403,"error":{"message":"Authentication is not enabled"},"extra_fields":{}}"#,
                ),
                (_, "/api/governance/virtual-keys") => write_response(
                    &mut stream,
                    200,
                    &[],
                    r#"{"virtual_key":{"id":"vk_test","name":"bear:test:123","value":"sk-bf-test"}}"#,
                ),
                (_, _) => write_response(&mut stream, 404, &[], r#"{"error":"not found"}"#),
            }
        }
    });

    (format!("http://{addr}/api"), records)
}

fn spawn_quota_validation_mock(
    accepted_mode: Option<BifrostVirtualKeyAuthMode>,
) -> (String, Arc<Mutex<Vec<RequestRecord>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock quota server");
    let addr = listener.local_addr().expect("mock quota server addr");
    let records = Arc::new(Mutex::new(Vec::new()));
    let records_for_thread = Arc::clone(&records);
    let expected_requests = if accepted_mode.is_some() { 1 } else { 3 };

    thread::spawn(move || {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().expect("accept mock quota request");
            let request = read_request(&mut stream);
            let record = request_record(&request);
            let recognized = match accepted_mode {
                Some(BifrostVirtualKeyAuthMode::XApiKey) => {
                    record.x_api_key.as_deref() == Some("sk-bf-test")
                }
                Some(BifrostVirtualKeyAuthMode::XBfVk) => {
                    record.x_bf_vk.as_deref() == Some("sk-bf-test")
                }
                Some(BifrostVirtualKeyAuthMode::Bearer) => {
                    record.authorization.as_deref() == Some("Bearer sk-bf-test")
                }
                None => false,
            };
            records_for_thread
                .lock()
                .expect("records mutex")
                .push(record);
            if recognized {
                write_response(
                    &mut stream,
                    200,
                    &[],
                    r#"{"virtual_key_name":"bear:test","is_active":true,"budgets":null,"rate_limit":null,"provider_configs":[],"model_configs":[]}"#,
                );
            } else {
                write_response(
                    &mut stream,
                    401,
                    &[],
                    r#"{"error":"virtual key not found"}"#,
                );
            }
        }
    });

    (format!("http://{addr}/api"), records)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).expect("read mock request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(headers_end) = find_headers_end(&buffer) {
            let headers = String::from_utf8_lossy(&buffer[..headers_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| line.split_once(':'))
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while buffer.len().saturating_sub(headers_end + 4) < content_length {
                let read = stream.read(&mut chunk).expect("read mock request body");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            break;
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn request_record(request: &str) -> RequestRecord {
    let mut lines = request.lines();
    let path = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_string();
    let mut authorization = None;
    let mut cookie = None;
    let mut x_api_key = None;
    let mut x_bf_vk = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.trim().to_string());
        } else if name.eq_ignore_ascii_case("cookie") {
            cookie = Some(value.trim().to_string());
        } else if name.eq_ignore_ascii_case("x-api-key") {
            x_api_key = Some(value.trim().to_string());
        } else if name.eq_ignore_ascii_case("x-bf-vk") {
            x_bf_vk = Some(value.trim().to_string());
        }
    }
    RequestRecord {
        path,
        authorization,
        cookie,
        x_api_key,
        x_bf_vk,
    }
}

fn write_response(stream: &mut TcpStream, status: u16, headers: &[(&str, &str)], body: &str) {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Status",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(name.as_ref());
        response.push_str(": ");
        response.push_str(value.as_ref());
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    response.push_str(body);
    stream
        .write_all(response.as_bytes())
        .expect("write mock response");
}

#[tokio::test]
async fn create_virtual_key_uses_bearer_token_login() {
    let (management_url, records) = spawn_bifrost_management_mock(LoginMode::BearerToken);
    let client = BifrostGovernanceClient::new(&test_config(management_url));

    let created = client
        .create_bear_virtual_key(uuid::Uuid::nil(), "test")
        .await
        .expect("create virtual key");

    assert_eq!(created.id, "vk_test");
    let records = records.lock().expect("records mutex");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].path, "/api/session/login");
    assert_eq!(records[1].path, "/api/governance/virtual-keys");
    assert_eq!(
        records[1].authorization.as_deref(),
        Some("Bearer bearer-token-123")
    );
    assert!(records[1].cookie.is_none());
}

#[tokio::test]
async fn create_virtual_key_uses_cookie_session_login() {
    let (management_url, records) = spawn_bifrost_management_mock(LoginMode::CookieSession);
    let client = BifrostGovernanceClient::new(&test_config(management_url));

    let created = client
        .create_bear_virtual_key(uuid::Uuid::nil(), "test")
        .await
        .expect("create virtual key");

    assert_eq!(created.value, "sk-bf-test");
    let records = records.lock().expect("records mutex");
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].path, "/api/governance/virtual-keys");
    assert_eq!(
        records[1].cookie.as_deref(),
        Some("bifrost_session=session-123")
    );
    assert!(records[1].authorization.is_none());
}

#[tokio::test]
async fn auth_disabled_login_error_includes_config_store_reset_hint() {
    let (management_url, records) = spawn_bifrost_management_mock(LoginMode::AuthDisabled);
    let client = BifrostGovernanceClient::new(&test_config(management_url));

    let err = client
        .create_bear_virtual_key(uuid::Uuid::nil(), "test")
        .await
        .expect_err("auth-disabled login should fail before create");
    let message = err.to_string();

    assert!(message.contains("Authentication is not enabled"));
    assert!(message.contains("governance.auth_config.is_enabled=true"));
    assert!(message.contains("/app/data/config.db"));
    let records = records.lock().expect("records mutex");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].path, "/api/session/login");
}

#[tokio::test]
async fn validate_virtual_key_value_accepts_x_api_key() {
    let (management_url, records) =
        spawn_quota_validation_mock(Some(BifrostVirtualKeyAuthMode::XApiKey));
    let client = BifrostGovernanceClient::new(&test_config(management_url));

    let validation = client
        .validate_virtual_key_value("sk-bf-test")
        .await
        .expect("validate virtual key");

    assert_eq!(validation.auth_mode, BifrostVirtualKeyAuthMode::XApiKey);
    let records = records.lock().expect("records mutex");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].path, "/api/governance/virtual-keys/quota");
    assert_eq!(records[0].x_api_key.as_deref(), Some("sk-bf-test"));
}

#[tokio::test]
async fn validate_virtual_key_value_reports_all_failed_header_modes() {
    let (management_url, records) = spawn_quota_validation_mock(None);
    let client = BifrostGovernanceClient::new(&test_config(management_url));

    let err = client
        .validate_virtual_key_value("sk-bf-test")
        .await
        .expect_err("validation should fail");
    let message = err.to_string();

    assert!(message.contains("x-api-key"));
    assert!(message.contains("x-bf-vk"));
    assert!(message.contains("bearer"));
    let records = records.lock().expect("records mutex");
    assert_eq!(records.len(), 3);
}
