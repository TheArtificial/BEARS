import os
import socket
import subprocess
import time
import uuid

import requests


def service_url(env_name, service_name, port):
    override = os.environ.get(env_name)
    if override:
        return override.rstrip("/")
    try:
        socket.gethostbyname(service_name)
        host = service_name
    except OSError:
        container_id = subprocess.check_output(
            ["docker", "compose", "ps", "-q", service_name],
            text=True,
            timeout=5,
        ).strip()
        host = subprocess.check_output(
            [
                "docker",
                "inspect",
                "-f",
                "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
                container_id,
            ],
            text=True,
            timeout=5,
        ).strip()
    return f"http://{host}:{port}"


DEN = service_url("BEARS_DEN_URL", "bears-den", 3000)
MEMFS_MANAGER = service_url("BEARS_MEMFS_MANAGER_URL", "bears-memfs-manager", 8285)
CODEPOOL = service_url("BEARS_CODEPOOL_URL", "bears-codepool", 3030)
API = os.environ.get("BEARS_API_URL", "").rstrip("/")
SEEDED_USERNAME = "alice"
SEEDED_PASSWORD = "Never deploy seed passwords."
SEEDED_BEAR_SLUG = "test-bear"
SEEDED_ACP_TOKEN = "bears_acp_smoke_known_token_for_dev_and_ci_only_000000000000"
LETTA = service_url("BEARS_LETTA_URL", "bears-letta", 8283)
LETTA_API_KEY = os.environ.get("LETTA_API_KEY") or os.environ.get(
    "LETTA_SERVER_PASS", "dev-placeholder"
)


def request_with_retries(method, url, **kwargs):
    session = kwargs.pop("session", requests)
    last_error = None
    for _ in range(20):
        try:
            response = session.request(method, url, **kwargs)
            if response.status_code < 500:
                return response
            last_error = AssertionError(
                f"{url} returned {response.status_code}: {response.text}"
            )
        except requests.RequestException as exc:
            last_error = exc
        time.sleep(2)
    raise AssertionError(f"request failed after retries: {url}: {last_error}")


def test_memfs_manager_health():
    response = request_with_retries("GET", f"{MEMFS_MANAGER}/health", timeout=5)
    assert response.status_code == 200


def test_den_reachable():
    response = request_with_retries("GET", f"{DEN}/health", timeout=5)
    assert response.status_code == 200


def test_pool_health():
    response = request_with_retries("GET", f"{CODEPOOL}/health", timeout=5)
    assert response.status_code == 200


def test_api_health_when_enabled():
    if not API:
        return
    response = request_with_retries("GET", f"{API}/health", timeout=5)
    assert response.status_code == 200


def test_acp_requires_bearer_token_when_api_enabled():
    if not API:
        return
    response = request_with_retries(
        "POST",
        f"{API}/acp/bears/{SEEDED_BEAR_SLUG}/sessions/smoke-session/prompt",
        json={"message": "hello", "client": "zed"},
        timeout=5,
    )
    assert response.status_code in (401, 404), response.text
    if response.status_code == 401:
        assert "error_code" in response.text


def parse_sse_data(response):
    events = []
    for frame in response.text.split("\n\n"):
        for line in frame.splitlines():
            if not line.startswith("data:"):
                continue
            raw = line[len("data:") :].strip()
            if raw and raw != "[DONE]":
                try:
                    events.append(__import__("json").loads(raw))
                except Exception:
                    pass
    return events


def stream_acp_prompt_events(session_id, payload, timeout=30):
    with requests.post(
        f"{API}/acp/bears/{SEEDED_BEAR_SLUG}/sessions/{session_id}/prompt",
        json=payload,
        headers={"Authorization": f"Bearer {SEEDED_ACP_TOKEN}"},
        timeout=timeout,
        stream=True,
    ) as response:
        assert response.status_code == 200, response.text
        for line in response.iter_lines(decode_unicode=True):
            if line is None or line == "" or not line.startswith("data:"):
                continue
            raw = line[len("data:") :].strip()
            if not raw or raw == "[DONE]":
                continue
            yield __import__("json").loads(raw)


def post_acp_prompt_until_conversation_resolved(session_id, payload, timeout=30):
    with requests.post(
        f"{API}/acp/bears/{SEEDED_BEAR_SLUG}/sessions/{session_id}/prompt",
        json=payload,
        headers={"Authorization": f"Bearer {SEEDED_ACP_TOKEN}"},
        timeout=timeout,
        stream=True,
    ) as response:
        assert response.status_code == 200, response.text
        for line in response.iter_lines(decode_unicode=True):
            if line is None:
                continue
            if line == "":
                continue
            if not line.startswith("data:"):
                continue
            raw = line[len("data:") :].strip()
            if not raw or raw == "[DONE]":
                continue
            event = __import__("json").loads(raw)
            if event.get("type") == "conversation_resolved" and event.get(
                "conversation_id"
            ):
                response.close()
                return event["conversation_id"]
        raise AssertionError("conversation_resolved not received")


def letta_headers():
    return {"Authorization": f"Bearer {LETTA_API_KEY}"}


def create_smoke_letta_agent():
    agent_id = f"agent-smoke-boundary-{uuid.uuid4()}"
    agent = request_with_retries(
        "POST",
        f"{LETTA}/v1/agents/",
        headers=letta_headers(),
        json={
            "name": f"Smoke Boundary {agent_id}",
            "memory_blocks": [
                {"label": "human", "value": "Smoke test human."},
                {"label": "persona", "value": "Smoke test pair agent."},
            ],
            "model": "letta/letta-free",
            "embedding": "letta/letta-free",
            "agent_type": "letta_v1_agent",
        },
        timeout=30,
    )
    assert agent.status_code in (200, 201), agent.text
    agent_body = agent.json()
    agent_id = agent_body.get("id") or agent_body.get("agent", {}).get("id")
    assert agent_id, agent.text

    return agent_id


def test_acp_pair_does_not_persist_runtime_context_in_letta_user_message():
    if not API:
        return
    create_smoke_letta_agent()
    marker = f"smoke-boundary-check-{int(time.time())}"
    session_id = f"smoke-boundary-{int(time.time())}"
    conversation_id = post_acp_prompt_until_conversation_resolved(
        session_id,
        {
            "message": marker,
            "conversation_id": f"new-smoke-boundary-{uuid.uuid4()}",
            "client": "zed",
            "client_context": {"cwd": "/workspace"},
        },
    )

    history = request_with_retries(
        "GET",
        f"{LETTA}/v1/conversations/{conversation_id}/messages?limit=20&order=desc",
        headers=letta_headers(),
        timeout=10,
    )
    assert history.status_code == 200, history.text
    body = history.json()
    raw_messages = (
        body
        if isinstance(body, list)
        else body.get("messages") or body.get("data") or []
    )
    user_texts = []
    for msg in raw_messages:
        inner = msg.get("message") if isinstance(msg.get("message"), dict) else msg
        message_type = (
            inner.get("message_type")
            or inner.get("type")
            or msg.get("message_type")
            or msg.get("type")
        )
        role = inner.get("role") or msg.get("role")
        if message_type not in ("user_message", "user") and role != "user":
            continue
        text = (
            inner.get("content")
            or inner.get("text")
            or inner.get("message")
            or msg.get("content")
            or msg.get("text")
            or msg.get("message")
        )
        if isinstance(text, str):
            user_texts.append(text)
    matching = [text for text in user_texts if marker in text]
    forbidden = [
        "<system-reminder",
        "<system_reminder",
        "ACP workflow state",
        "AUTHORITATIVE WORKFLOW STATE",
        "Den workboard context",
        "Trusted ACP session mode this turn",
    ]
    if matching:
        text = matching[0]
        assert text.strip() == marker
        for needle in forbidden:
            assert needle not in text
        return

    # Some Letta error paths create the conversation and expose it to Den/ACP
    # before the user message is persisted in the conversation message listing.
    # This smoke test is specifically guarding the clean user-message boundary:
    # if the marker has not been persisted at all, still assert that no persisted
    # user message contains Den runtime scaffolding.
    assert user_texts == [], (
        f"marker {marker!r} not found, but unexpected user messages were present: {user_texts!r}"
    )
    serialized_history = __import__("json").dumps(raw_messages)
    assert marker not in serialized_history
    for needle in forbidden:
        assert needle not in serialized_history


def test_seeded_user_can_open_seeded_bear_page():
    session = requests.Session()
    login = request_with_retries(
        "POST",
        f"{DEN}/login/password",
        session=session,
        data={"username": SEEDED_USERNAME, "password": SEEDED_PASSWORD},
        timeout=5,
        allow_redirects=False,
    )
    assert login.status_code in (302, 303), login.text

    response = session.get(f"{DEN}/bear/{SEEDED_BEAR_SLUG}", timeout=5)
    assert response.status_code == 200, response.text
    assert "Test Bear" in response.text


def test_acp_tool_result_replay_continues_and_is_idempotent_when_api_enabled():
    if not API:
        return

    session_id = f"smoke-tool-replay-{int(time.time())}-{uuid.uuid4().hex[:8]}"
    prompt = {
        "message": "Please read /workspace/README.md using the available file tools and then summarize it in one sentence.",
        "conversation_id": f"new-smoke-tool-replay-{uuid.uuid4()}",
        "client": "zed",
        "client_context": {"cwd": "/workspace"},
    }

    tool_request = None
    conversation_id = None
    for event in stream_acp_prompt_events(session_id, prompt, timeout=60):
        if event.get("type") == "conversation_resolved" and event.get("conversation_id"):
            conversation_id = event["conversation_id"]
        if event.get("type") == "tool_request":
            tool_request = event
            break

    assert tool_request, "expected tool_request event from ACP prompt stream"
    tool_call_id = tool_request.get("tool_call_id")
    assert tool_call_id, tool_request
    tool_name = tool_request.get("name") or tool_request.get("tool_name")
    assert tool_name, tool_request
    arguments = tool_request.get("arguments") or {}
    assert "README.md" in __import__("json").dumps(arguments)

    first_response = request_with_retries(
        "POST",
        f"{API}/acp/bears/{SEEDED_BEAR_SLUG}/sessions/{session_id}/tool-results/{tool_call_id}",
        headers={"Authorization": f"Bearer {SEEDED_ACP_TOKEN}"},
        json={
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "status": "ok",
            "content": "# Smoke README\n\nThis is a replay smoke test result.",
            "structured_content": {"path": "/workspace/README.md", "kind": "file_excerpt"},
            "diagnostic": {"phase": "smoke-first"},
        },
        timeout=30,
    )
    assert first_response.status_code == 200, first_response.text
    first_json = first_response.json()
    assert first_json["accepted"] is True
    assert first_json["settlement"] in ("accepted", "delivered", "pending_continuation", None)

    replay_response = request_with_retries(
        "POST",
        f"{API}/acp/bears/{SEEDED_BEAR_SLUG}/sessions/{session_id}/tool-results/{tool_call_id}",
        headers={"Authorization": f"Bearer {SEEDED_ACP_TOKEN}"},
        json={
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "status": "ok",
            "content": "# Smoke README\n\nThis is a replay smoke test result.",
            "structured_content": {"path": "/workspace/README.md", "kind": "file_excerpt"},
            "diagnostic": {"phase": "smoke-first"},
        },
        timeout=30,
    )
    assert replay_response.status_code == 200, replay_response.text
    replay_json = replay_response.json()
    assert replay_json["accepted"] is True
    assert replay_json["reason"] == "duplicate_result_ignored"
    assert replay_json["settlement"] == "already_settled"
    assert replay_json["diagnostic"]["tool_call_id"] == tool_call_id
    assert replay_json["diagnostic"]["status"] == "ok"

    if conversation_id:
        history = request_with_retries(
            "GET",
            f"{API}/acp/bears/{SEEDED_BEAR_SLUG}/conversations/{conversation_id}/history",
            headers={"Authorization": f"Bearer {SEEDED_ACP_TOKEN}"},
            timeout=30,
        )
        assert history.status_code == 200, history.text
        body = history.json()
        messages = body.get("messages") or []
        assert any(msg.get("role") == "user" for msg in messages), body
