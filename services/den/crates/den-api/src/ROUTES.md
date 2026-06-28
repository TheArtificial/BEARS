# API service routes

Axum routes for the API server (`RUN_API=true`).

## Top-level

- `GET /health` — API liveness.
- `GET /version` — JSON build identity.
- `GET /healthcheck` — legacy liveness alias.
- `GET /health/ready` — database readiness check.
- `GET /api-docs/openapi.json` — OpenAPI document.

## OAuth

- `GET|POST /oauth/*` — OAuth 2.0 authorization server. See [`oauth/README.md`](oauth/README.md).

## v1.0 API

- `GET /v1.0/me` — bearer-token authenticated profile endpoint.
- `GET|POST /v1.0/oauth/*` — token management endpoints.

## ACP gateway

- `GET /acp/bears/{slug}/sessions` — bearer-token authenticated ACP session binding list. Requires `RUN_API=true` and a bearer token with `acp:chat` scope.
- `GET /acp/bears/{slug}/sessions/{session_id}` — bearer-token authenticated ACP session binding detail. Response uses `runtime_session_id` (not historical `codepool_session_id`).
- `POST /acp/bears/{slug}/sessions/{session_id}/prompt` — API-only bearer-token authenticated gateway for ACP adapter clients. Requires `RUN_API=true`, a provisioned `bear_profile_bindings(profile='pair')`, and a bearer token with `acp:chat` scope.
- `POST /acp/bears/{slug}/sessions/{session_id}/cancel` — cancels active native runtime work when possible; otherwise returns a diagnostic response.
- `POST /acp/bears/{slug}/sessions/{session_id}/close` — marks the ACP session binding closed and archives the resolved Den conversation where possible.
- `GET /acp/bears/{slug}/conversations` — lists conversations for the Bear's pair role agent.
- `GET /acp/bears/{slug}/conversations/{conversation_id}/history` — loads conversation history for the Bear's pair role agent.
- `GET /acp/bears/{slug}/auth-check` — validates bearer token and membership for the Bear.
