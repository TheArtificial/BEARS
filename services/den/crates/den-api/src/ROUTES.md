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

## BearWire

BearWire is mounted by the binary composition root under `/bearwire`; it is not part of `den-api` itself.
