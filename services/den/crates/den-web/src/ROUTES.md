# Web module routes

Axum routes for the web server (`RUN_WEB=true`). Update this file when you add or remove routes.

See also `src/web/WEB_UI_FIXTURES.md` for the feature-gated browser/UI fixture workflow used for
real-page smoke testing in development.

## Top-level (`src/web/mod.rs`)

- `GET /health` — liveness (BEARS Phase 1 M0 canonical path)
- `GET /version` — JSON build identity (`service`, `version` from Cargo.toml, `built_at_utc`, `git_sha` from `GIT_SHA` Docker build-arg or `unknown`)
- `GET /healthcheck` — liveness (legacy alias)
- `GET /health/ready` — readiness (DB ping)
- `GET /metrics` — Prometheus text exposition (in-memory counters for chat send outcomes; scrape on the internal network; no auth — protect with firewall / reverse proxy as for other metrics endpoints)
- `GET /status` — **BEARS stack** status page: aggregate health probes plus **deployed vs GHCR** when `GITHUB_PACKAGES_TOKEN` + `GHCR_PACKAGES_OWNER` are set
- `GET /status.json` — combined JSON (`health`, `den_version`, optional `ghcr_*`) — **503** if any health check is `fail`
- `GET /design` — CSS fixture page for text, forms, and two-column layout
- `GET /design/chat` — static chat UI fixture for iterating on chat styling
- `GET /manifest.json` — Web App Manifest (`APP_DISPLAY_NAME`, `APP_SLUG`, icons)
- `GET /assets/*` — static assets (memory-serve)
- `GET /*` — fallback 404 (`src/web/public.rs`) for unmatched paths

## Authenticated user (`src/web/user/mod.rs`)

- `GET|POST /settings/*` — profile / email settings (login required)
- `GET|POST /account/*` — registration, account view, password
- `GET /login`, `POST /login/password`, `GET /logout`, `GET /su/{id}` — session (`src/web/user/session.rs`)

## Home (`src/web/home.rs`)

- `GET /` — marketing home when logged out; logged-in verified users with bears see `dashboard.html`; verified users with no bears redirect to `/onboarding/first-bear`; unverified users redirect to email verify

## Onboarding (`src/web/onboarding.rs`)

- `GET|POST /onboarding/first-bear` — first Bear setup flow for verified users with no Bear memberships; creates a stance-aware `context_profile`, provisions native stance bindings, and redirects to chat

## Bear management (`src/web/bear/settings.rs`, `src/web/bear/manage.rs`)

Member-facing bear management at `/bear/{slug}/…` (read for members, write for bear admins), organized by ownership: **Yours** (identity, memory, skills — travels with the Bear) and **This Den** (tools, connections, resources, activity, people — stays here).

- `GET /bear/{slug}/overview` — health, pending-review call to action, recent activity; wide viewports disclose memory statistics and activity-over-time (CSS only)
- `GET /bear/{slug}/identity` — identity & charter summary; links to edit forms and per-stance models
- `GET /bear/{slug}/skills` — owned procedures (honest placeholder until Skills land)
- `GET /bear/{slug}/tools` — per-stance tool roster with origin (built-in / armature-local; MCP when it lands)
- `GET /bear/{slug}/connections` — editor (armature) code token; provider connections when they land
- `GET /bear/{slug}/resources` — the web as a resource under policy (sources/approvals/fetches; POST actions as before), internal resources noted
- `GET /bear/{slug}/activity` — activity hub: conversations stream (jobs and Cabinet when they land); `GET /bear/{slug}/conversations/{conversation_id}` — transcript detail
- `GET /bear/{slug}/people` — membership; bear admins grant/revoke via POST actions
- `GET /bear/{slug}/portability` — bundle export (`GET /bear/{slug}/export.bear`), import (`POST /bears/import`), what-moves/what-stays
- `GET /bear/{slug}/context` — prompt assembly, layer by layer: compiled stance prompts, standing notes (durable prompt-memory blocks, humanized), projected memory, recall status, conversation window, tool surface
- Internals (kept reachable): `GET /bear/{slug}/stances/{stance}` (stance detail + model POSTs; linked from identity/models), `GET|POST /bear/{slug}/models`, `GET /bear/{slug}/advanced` (diagnostics incl. stance-binding status, provision action)
- Retired paths redirect: `/access` → `/people`, `/policy` → `/resources`, `/stances` (list) → `/advanced`, `/persona` → `/context`; `/conversations` remains as an alias of the activity stream

## Bear memory & entities (`src/bear_memory.rs`)

- `GET /bear/{slug}/memory` — memory dashboard ("how much memory": counts by kind/role, recall coverage, entity summary, recent additions, governance)
- `POST /bear/{slug}/memory/import-legacy` — legacy archived-bundle import route; stages a bundle at `<BEAR_SQLITE_DATA_DIR>/imports/{bear_id}/`, imports legacy memory heads into per-Bear SQLite, and redirects back to the memory dashboard with success/error notices
- `GET /bear/{slug}/memory/recent` — recent additions feed (newest records across all roles)
- `GET /bear/{slug}/memory/search?q=&mode=` — search (keyword always; `mode=semantic` uses the recall index when configured)
- `GET|POST /bear/{slug}/memory/browse` — library of logical paths grouped by scope; POST deletes/requests review for selected paths (bear admins)
- `GET /bear/{slug}/memory/records/{memory_id}` — single entry: content, history (versions at path), referenced entities, recall status
- `GET|POST /bear/{slug}/memory/proposals/{proposal_id}` — memory review proposal detail and resolution
- `GET /bear/{slug}/entities?type=` — entity library (ADR-0042), optionally filtered by type
- `GET /bear/{slug}/entities/{entity_id}` — entity detail: handles, linked memory records, cross-links

## Member bear management (`src/web/bear_management.rs`)

- `GET|POST /bears/new` — create a bear; creator is granted `user_bear.role = admin`
- `GET /bear/{slug}/details` — permanent redirect to `/bear/{slug}/overview`
- `GET /bear/{slug}/details/{*rest}` — permanent redirects to canonical `/bear/{slug}/…` paths (legacy `roles/` → `stances/`)
- `GET /bear/{slug}/edit` — redirect to `/bear/{slug}/edit/overview`
- `GET|POST /bear/{slug}/edit/overview` — edit slug, name, description; delete bear form
- `GET|POST /bear/{slug}/edit/prompt` — edit system prompt (bear admins)
- `GET|POST /bear/{slug}/edit/configuration` — edit default model only via Bifrost catalog (bear admins)
- `GET|POST /bear/{slug}/code-token` — ACP code token for pair profile
- `GET /bear/{slug}/memory/browse/runtime-blocks` — permanent redirect to `/bear/{slug}/advanced` (deprecated)
- `GET /bear/{slug}/memory/browse/proposals/{id}` — permanent redirect to `/bear/{slug}/memory/proposals/{id}`
- `POST /bear/{slug}/delete` — delete bear row (bear admins only)
- `POST /bear/{slug}/members/add`, `POST /bear/{slug}/members/remove` — legacy membership actions

## End-user chat (Phase 1 — same origin as web)

- `GET /bear/{slug}` — Deep Chat view for a single bear the user may access (membership-checked; `src/web/templates/bear_chat.html`, handler in `src/web/bear_chat.rs`). Registered with trailing-slash redirect (`/bear/{slug}/` → `/bear/{slug}`) so links like `/bear/{slug}/?conversation_id=…` from the details UI resolve.
- `GET /v1/bears` — JSON list of bears the signed-in user may use (membership-filtered; includes `is_bear_admin`) (`src/web/v1/mod.rs`).
- `GET /v1/chat/conversations` — query `bear_id` (required). Membership-checked; returns `{ "conversations": [ { "id", "title", "last_message_at" } ] }` from Den-owned conversation persistence.
- `PATCH /v1/chat/conversations/{conversation_id}` — JSON body `bear_id` plus optional `title` and/or `archived`; membership-checked wrapper for Den-owned conversation metadata.
- `GET /v1/chat/history` — query `bear_id` (required), optional `conversation_id`, optional `before`, optional `limit` (default 50, max 100). Membership-checked; loads Den-owned conversation history for Deep Chat `loadHistory`.
- `POST /v1/chat/send` — JSON body `bear_id`, `message`, optional `conversation_id`. Membership-checked; runs the Den-native chat loop through Bifrost. Each request gets a UUID **`X-Request-Id`** on the response (SSE success or JSON error). Failures return **`application/json`** `{ "error": "…", "request_id": "…" }` (not HTML). The browser parses `data:` lines and shows `reasoning_message`, `assistant_message`, and `error_message` payloads in Deep Chat (see `bear_chat.html`).

`/v1/*` uses `login_required!(…)` (same session as the rest of the web app).

## Admin (`src/web/admin/mod.rs`)

- `GET /admin/` — admin menu
- `GET|POST /admin/users/*` — user management
- `GET|POST /admin/bears/*` — bear registry (create bear with prompt/model fields and native stance provisioning defaults)
- `GET /admin/bears/{id}` — redirects to member-facing `/bear/{slug}/…` profile/settings pages
- `GET|POST /admin/bears/{id}/edit` — legacy redirects to member-facing edit pages
- `GET|POST /admin/membership/*` — list and grant `user_bear` membership
- `GET|POST /admin/api/*` — JSON admin API (bears, membership; operator session cookie)
- `GET|POST /admin/oauth_clients/*` — OAuth client CRUD, PKCE test
- `GET|POST /admin/oauth_tokens/*` — token admin

All `/admin/*` routes use `permission_required!(…, "admin")`.

## API service (separate router)

The standalone API (`RUN_API=true`) is built in `src/api/service.rs` — see `src/api/` and `src/api/oauth/README.md`, not this file.
