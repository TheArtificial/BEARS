# BearWire armature wire — implementation plan

**Status:** Draft  
**Date:** 2026-06-16  
**Decision:** [ADR-0034: BearWire as the Den ↔ armature wire](../decisions/adr-0034-bearwire-as-den-armature-wire.md)  
**Specs:** [BearWire JSON specification](../architecture/bearwire-json-spec.md), [BearWire Rust design](../architecture/bearwire-rust-design.md)

## Purpose

Replace the bespoke **adapter-SSE** + `/acp/**` HTTP dialect between Den and trusted armatures with **BearWire v1** — while keeping user-visible behavior stable and allowing `bears-acp-adapter` to own all ACP stdio translation.

This plan is the delivery checklist for ADR-0034. It assumes the protocol-neutral core work (neutral turn/session names, `GatewayEvent`, golden traces) is either done or in flight; this plan starts at the **wire boundary**.

## End state

```text
Editor ──ACP stdio──► bears-acp-adapter
                           │  BearWire v1 (HTTP RPC + SSE events)
                           ▼
                       den-bearwire edge  (evolved from den-acp)
                           │  RuntimeSemanticEvent / den-runtime
                           ▼
                       agent loop, memory, bears
```

- No adapter-SSE `type` strings on the Den ↔ armature hop.
- `/acp/**` removed or thin-shimmed to BearWire internally.
- `den-acp` renamed or superseded by `den-bearwire` (name optional; boundary is what matters).

## Principles

1. **Semantic parity before transport churn** — lock event mapping with tests before switching the adapter default.
2. **Parallel operation** — legacy `/acp` + adapter-SSE stays until `bears-acp-adapter` ships BearWire client support.
3. **Serializable wire types** — no `oneshot` channels or in-process-only fields on BearWire structs.
4. **Armature owns ACP** — no new ACP-shaped JSON in `den-runtime`.

## Current inventory (baseline)

### HTTP control plane (`/acp/bears/{slug}/…`)

| Route | Role today |
| --- | --- |
| `GET …/sessions` | List session bindings |
| `GET …/sessions/{id}` | Session detail + runtime snapshot |
| `GET …/sessions/{id}/runtime` | Runtime health / waiting state |
| `GET …/sessions/{id}/prompt-memory` | Prompt memory read |
| `POST …/sessions/{id}/prompt` | Start turn (SSE response stream) |
| `POST …/sessions/{id}/tool-results/{tool_call_id}` | Deliver client tool result |
| `POST …/sessions/{id}/permissions/{permission_id}` | Permission decision |
| `POST …/sessions/{id}/mode` | Plan / write mode |
| `POST …/sessions/{id}/adapter-environment` | Adapter capability report |
| `POST …/sessions/{id}/cancel` | Cancel active run |
| `POST …/sessions/{id}/close` | Close session |
| `POST …/sessions/{id}/compact` | Compaction trigger |
| `GET …/conversations` | Conversation list |
| `GET …/conversations/{id}/history` | History |
| `GET …/auth-check` | Token + membership check |

### Internal (not BearWire public)

| Route | Role |
| --- | --- |
| `POST /internal/den-tools/invoke` | Server-executed Den tools (adapter calls with internal token) |

### Event stream (adapter-SSE today)

`gateway_event_to_adapter_sse` in `den-runtime` emits `data: {"type":"<adapter_sse_type>",…}\n\n`.

| Adapter-SSE `type` | Source `GatewayEvent` variant |
| --- | --- |
| `assistant_text_delta` | `AssistantTextDelta` |
| `status_text` | `StatusText` |
| `tool_request` | `ToolRequest` |
| `permission_request` | `PermissionRequest` |
| `turn_complete` | `TurnComplete` |
| `turn_result` | `TurnResult` |
| `error` | `Error` |
| `plan_update` | `PlanUpdate*` |
| `mode_update` | `ModeUpdate` |
| `conversation_resolved` | `ConversationResolved` |
| `session_info_update` | `SessionInfoUpdate` |

## Phase 0 — Mapping lock (no wire change)

**Goal:** Pin the adapter-SSE → BearWire mapping in tests and docs.

| Task | Owner | Done when |
| --- | --- | --- |
| Add `docs/architecture/bearwire-adapter-sse-migration.md` table (or extend JSON spec §Compatibility) with every adapter-SSE type mapped to BearWire `type` + payload notes | docs | Table reviewed; matches ADR-0034 §6 |
| Add `bearwire_projection/bearwire_wire_migration_tests.rs` in `den-runtime` | runtime | For each golden `RuntimeSemanticEvent` fixture, assert both adapter-SSE JSON and BearWire `event` notification JSON (stub BearWire serializer) |
| Document v1 HTTP+SSE binding in `bearwire-json-spec.md` (§Transport binding — v1 HTTP profile) | docs | Spec describes `POST /bearwire/v1/rpc` + SSE `event` stream |

**Exit gate:** Tests compile; mapping table complete; no production wire change.

## Phase 1 — Wire types + projection in core

**Goal:** Introduce serializable BearWire wire types and projection from `RuntimeSemanticEvent`.

| Task | Location | Notes |
| --- | --- | --- |
| Add `den-core` or `den-runtime` module `bearwire::wire` | `crates/den-core/src/bearwire/` or `den-runtime/src/bearwire/` | Follow [BearWire Rust design](../architecture/bearwire-rust-design.md): `EventEnvelope`, `EventId`, `RunId`, payload enums |
| Implement `runtime_semantic_event_to_bearwire_events()` | beside existing `bearwire_projection` | Parallel to `runtime_semantic_event_to_bearwire_gateway_events`; no channels on output |
| Implement `bearwire_event_to_json_rpc_notification()` | same module | JSON-RPC `method: "event"` per JSON spec |
| Keep `GatewayEvent` for in-process orchestration only | `gateway_events.rs` | Document as transitional; new armature path bypasses adapter-SSE |
| Golden tests | `golden_traces_tests.rs` + new migration tests | Same scenarios produce equivalent *semantic* outcomes |

**Exit gate:** `cargo test -p den-runtime bearwire` green; golden traces extended.

## Phase 2 — BearWire HTTP edge (parallel to `/acp`)

**Goal:** Serve BearWire v1 alongside legacy routes.

| Task | Location | Notes |
| --- | --- | --- |
| Add `den-bearwire` crate **or** `den-acp/src/bearwire/` module | edge crate | Prefer new module first; rename crate when `/acp` is gone |
| `POST /bearwire/v1/rpc` — JSON-RPC dispatcher | edge router | Methods: `initialize`, `session.open`, `session.resume`, `session.close`, `session.state`, `run.start`, `run.cancel`, `client.tool.result`, `client.permission.result`, `resource.update` |
| `GET /bearwire/v1/sessions/{session_id}/events` — SSE | edge router | Stream `event` notifications; `sequence` monotonic per session |
| Wire auth | reuse `den-oauth` bearer + ACP token paths | Same scopes as `/acp/auth-check` |
| Map handlers to existing den-runtime / session store calls | reuse `den-acp` handler bodies | Thin RPC façade over current logic |
| Binary composition | `services/den/src/lib.rs` | Mount `/bearwire` peer router (same pattern as post-ADR-0043 composition) |
| Feature flag | `config.bearwire_enabled` or extend `acp_gateway_enabled` | Den advertises BearWire in `initialize` result when enabled |

**Exit gate:** Integration test: RPC `run.start` → SSE receives `message.delta` + `run.completed` for a trivial prompt (mock provider).

## Phase 3 — `bears-acp-adapter` BearWire client

**Goal:** Armature speaks BearWire to Den; translates to ACP stdio locally.

| Task | Location | Notes |
| --- | --- | --- |
| BearWire HTTP client (RPC + SSE consumer) | `tools/bears-acp-adapter` | Negotiate version in `initialize`; fall back to legacy `/acp` if Den < v1 |
| Move any remaining ACP-specific descriptor framing | adapter | Server sends neutral tool policy; adapter adds ACP wire methods for editor |
| Dual-mode operation | adapter config | `BEARS_BEARWIRE=1` or auto-detect from `initialize` |
| Parity test suite | adapter + den integration | Same editor session against legacy and BearWire backends |

**Exit gate:** Zed/Cursor smoke test against Den with BearWire enabled; golden semantic parity with legacy path.

## Phase 4 — Deprecate adapter-SSE and `/acp/**`

**Goal:** Single armature wire.

| Task | Notes |
| --- | --- |
| Default adapter to BearWire | Legacy path behind `BEARS_LEGACY_ACP_HTTP=1` for one release |
| Remove `gateway_event_to_adapter_sse` from hot path | Keep function behind `#[deprecated]` until adapter release ships |
| Redirect or 410 `/acp/**` routes | Or internal shim: `/acp/prompt` → `run.start` RPC (optional compatibility layer) |
| Rename `den-acp` → `den-bearwire` | Crate + docs; `ACP_GATEWAY_ENABLED` → `BEARWIRE_ENABLED` (env alias kept) |
| Update `ROUTES.md`, deploy docs, `.env.example` | |

**Exit gate:** No production adapter release depends on adapter-SSE; `/acp` documented as removed.

## Phase 5 — WebSocket transport (v2, optional)

**Goal:** Align transport with JSON spec preferred binding.

| Task | Notes |
| --- | --- |
| `wss://<den>/bearwire/v1` JSON-RPC bidirectional | Same methods + `event` notifications on socket |
| Adapter prefers WebSocket when available | HTTP+SSE remains fallback |
| Load balancer / Coolify notes | Sticky sessions or SSE-friendly proxy for v1 |

**Exit gate:** Adapter uses WebSocket in CI; HTTP+SSE still supported.

## Control-method mapping (complete)

| Legacy HTTP | BearWire method | Notes |
| --- | --- | --- |
| `GET …/auth-check` | `initialize` (subset) or preflight on `session.open` | Returns bearer validity + bear membership |
| `GET …/sessions` | `session.state` (list) | Pagination params in RPC |
| `GET …/sessions/{id}` | `session.state` | |
| `POST …/adapter-environment` | `resource.update` | `kind: "acp_adapter"` capabilities |
| `POST …/mode` | `session.state` + plan side effects | Plan mode is core; wire is state update |
| `POST …/prompt` | `run.start` | Returns `run_id`; events on SSE stream |
| `POST …/tool-results/{id}` | `client.tool.result` | Resumes paused run |
| `POST …/permissions/{id}` | `client.permission.result` | |
| `POST …/cancel` | `run.cancel` | |
| `POST …/close` | `session.close` | |
| `POST …/compact` | `run.start` with `kind: "compaction"` or dedicated `run.start` params | TBD in JSON spec |
| `GET …/history` | `session.state` or `memory.read` (future) | v1: keep as RPC wrapper over existing handler |

## Event mapping (complete)

| Adapter-SSE | BearWire `type` | Payload notes |
| --- | --- | --- |
| `assistant_text_delta` | `message.delta` | `text`, `message_id`, `run_id` subjects |
| `status_text` | `run.progress` | `kind: "status_text"`, `text` |
| `tool_request` (approval required) | `tool_call.blocked` | `reason: "permission_required"` + tool args |
| `tool_request` (no approval) | `tool_call.requested` | Delegation to armature |
| `permission_request` | `permission.requested` | |
| `turn_complete` | `run.completed` | `outcome: "ok"` |
| `turn_result` (ok) | `run.completed` | Merge `turn_result` diagnostics into payload |
| `turn_result` (error) | `run.failed` | `reason`, `retryable`, diagnostics |
| `error` (terminal) | `run.failed` | |
| `error` (non-terminal) | `run.warning` or `diagnostic.reported` | Classify in projection layer |
| `conversation_resolved` | `session.bound` | `conversation_id` as resource ref |
| `mode_update` | `session.state` | `mode` field |
| `plan_update` | `resource.updated` | `kind: "plan"` |
| `session_info_update` | `session.state` | title/meta |

## Test strategy

| Layer | What |
| --- | --- |
| Unit | `runtime_semantic_event` → BearWire JSON snapshots |
| Golden | Existing `golden_traces_tests` + BearWire SSE output |
| Integration | `den` test harness: RPC + SSE against test DB |
| Adapter | Parity tests: legacy HTTP vs BearWire for same prompt fixtures |
| Manual | Zed session: read file, tool approval, cancel |

## Risks

| Risk | Mitigation |
| --- | --- |
| Adapter-SSE and BearWire diverge during parallel run | Phase 0 mapping tests; single semantic source (`RuntimeSemanticEvent`) |
| WebSocket delayed indefinitely | v1 HTTP+SSE is full BearWire semantics, not a throwaway format |
| Large adapter change | Dual-mode + feature flag; legacy default until Phase 4 |
| `GatewayEvent` channels entangled with SSE | Phase 1 wire types bypass channels; handlers attach settlement server-side |

## Out of scope

- Public third-party BearWire API
- Replacing `/v1.0` REST or web UI channels (they may consume BearWire internally later)
- Renaming Postgres `acp_sessions` table (orthogonal DB migration)

## Milestone summary

| Phase | Milestone | User-visible |
| --- | --- | --- |
| 0 | Mapping locked in tests | None |
| 1 | BearWire types in den-runtime | None |
| 2 | `/bearwire/v1` served | None (flag off) |
| 3 | Adapter BearWire client | Opt-in via env |
| 4 | Legacy removed | Default path |
| 5 | WebSocket | Faster reconnect (optional) |

## Related documents

- [ACP Runtime Contract](../architecture/acp-runtime-contract.md) — update when `/acp` is shimmed
- [ADR-0003 session bindings](../decisions/adr-0003-acp-session-bindings.md) — session store unchanged; only wire changes
- [ACP Direct Local Tool Runtime Plan](archives/ACP_CLIENT_TOOL_RELAY_PLAN.md) — armature/local tool boundary (still valid)
