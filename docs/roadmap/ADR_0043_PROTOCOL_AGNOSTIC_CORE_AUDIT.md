# ADR-0043 Classification Audit — `acp_*` symbols in `den-runtime` / `den-core`

**Status:** Implemented (see §Implementation outcome)
**Date:** 2026-06-15
**Source decision:** [ADR-0043 — ACP is an edge adapter; the Den runtime is protocol-agnostic](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)

This is the **mandatory first step** of ADR-0043: enumerate every `acp_*` / `Acp*` symbol in the two core crates, bucket each as *core (rename, keep)* or *wire (move to the ACP adapter)*, and record the target name/home. It becomes the rename-and-relocate checklist. No code changes are part of this document.

## Scope

- **In scope:** `services/den/crates/den-runtime` and `services/den/crates/den-core` (the crates that must be protocol-agnostic).
- **Out of scope (but tracked):** `den-acp` (the adapter — destination of wire moves) and `den-api` (must stop depending on `den-acp` for shared state). Covered in §Cross-cutting.
- **Not part of the audit:** behavior changes. This is rename/relocate only, behind the test net in §Safety net.

## Legend

| Bucket | Meaning | Destination |
|---|---|---|
| **C** | Core concept wearing ACP clothes → rename to neutral vocabulary | stays in `den-runtime` / `den-core` |
| **W** | Genuine wire concept → relocate | `den-acp` adapter |
| **S** | Split — part core, part wire | both (most stays core) |

## Key finding that anchors the buckets

A **neutral semantic-event seam already exists** in the core: `runtime::contracts::{RuntimeSemanticEvent, RuntimeStreamEvent}` plus `runtime::bearwire_projection` (`runtime_stream_event_to_bearwire_sse`, `runtime_semantic_event_to_bearwire_gateway_events`). This is the canonical BearWire seam (ADR-0029/0030). Therefore `acp_events::AcpGatewayEvent` is a **parallel, ACP-flavored event model** layered on top — it is the ACP *projection*, not the canonical model. That makes `acp_events` a clean **wire** move and confirms the core already has the neutral event vocabulary the ADR calls for.

## Module-by-module classification (`den-runtime`)

### 1. `acp_turn_controller.rs` (249 `acp` hits) — **C** (pure core)

The turn-lifecycle state machine. Nothing here is ACP-specific; the only "ACP" is that turns are keyed by a client session id (already a core `SessionId`). **Strongest example of "core wearing ACP clothes."**

| Current symbol | Bucket | Target |
|---|---|---|
| `AcpTurnPhase`, `AcpToolExecutionRoute`, `AcpObligationStatus`, `AcpTerminalStatus`, `AcpTerminalReason`, `AcpTerminalOutcome` | C | `TurnPhase`, `ToolExecutionRoute`, `ObligationStatus`, `TerminalStatus`, `TerminalReason`, `TerminalOutcome` |
| `AcpToolObligation`, `AcpToolResultDisposition`, `AcpTurnStatusSnapshot`, `AcpTurnStatusUpdate` | C | drop `Acp` prefix |
| `AcpActiveTurnCancelRegistry` / `…Registration` / `…Handle` (+ `register`, `cancel_session`, `record_run_id`, `active_for_session`, `runtime_snapshot_for_session`) | C | `ActiveTurnCancelRegistry` etc. (`acp_session_id` arg → `session_id`) |
| `AcpTurnController` + all `on_*` methods (`on_stream_started`, `on_tool_request`, `on_den_tool_settled`, `on_adapter_tool_result`, `on_tool_timeout`, `on_requires_approval_stop`, `on_stream_end`, `on_stream_error`, `on_cancel`, …) | C | `TurnController` |
| module `acp_turn_controller` | C | `turn_controller` |

### 2. `acp_tool_turns.rs` (127) — **C** (pure core)

Tool-turn coordination / continuation / settlement. Core control flow.

| Current | Bucket | Target |
|---|---|---|
| `AcpToolTurnCoordinator`, `AcpActiveTurn`, `AcpActiveTurnGuard`, `AcpPendingToolTurn`, `AcpToolResultRequest`, `AcpToolResultDelivery`, `AcpToolTurnRegistration`, `AcpToolTurnCleanupSummary`, `AcpToolSettlementSummary`, `AcpSettledToolResult` | C | drop `Acp` prefix |
| `PrepareRuntimeContinuationError`, `PreparedRuntimeContinuation` (already neutral) | C | keep |
| methods: `acquire_active_turn`, `register`, `deliver_result`, `prepare_runtime_continuation`, `settle_after_result`, `cleanup_*`, `recently_settled`, … | C | unchanged names |
| module `acp_tool_turns` | C | `tool_turns` |

### 3. `acp_turn_runner.rs` (23) — **C** (core, with one renamed constant)

Turn start/continue request inputs + conversation materialization.

| Current | Bucket | Target |
|---|---|---|
| `AcpTurnStartRequest`, `AcpTurnContinueRequest`, `AcpTurnStreamContext`, `AcpRuntimeMaterializationResult` | C | `TurnStartRequest`, `TurnContinueRequest`, `TurnStreamContext`, `RuntimeMaterializationResult` |
| `default_acp_tool_continue_stream_context`, `materialize_acp_runtime_conversation_if_needed`, `looks_like_runtime_waiting_for_approval_error` | C | drop `acp` infix |
| `ACP_STALE_APPROVAL_RECOVERY_DENIAL_REASON` (const; message text mentions "ACP approval") | C | `STALE_APPROVAL_RECOVERY_DENIAL_REASON`; soften message to "client" wording |
| module `acp_turn_runner` | C | `turn_runner` |

### 4. `acp_plan_mode.rs` (88) — **C** (capability core; supersedes ADR illustrative table)

> The ADR-0043 illustrative table tentatively listed `acp_plan_mode` as *wire*. The audit **overrides** that: plan-mode is a **Den capability** (plan-then-act) with a DB-backed state machine and a tool surface that **already lives in core** (`den-core/src/tools/plan_mode/`). Only the *projection of plan entries into ACP SSE* is wire, and that already lives in the adapter (`den-acp/.../stream/plan.rs`).

| Current | Bucket | Target |
|---|---|---|
| `AcpPlanModeState`, `AcpPlanModeRequestedBy`, `AcpPlanModeSessionRow`, `EnterPlanModeParams`, `SubmitPlanModeParams` | C | drop `Acp` prefix |
| `enter_plan_mode`, `submit_plan_artifact`, `approve_plan_mode`, `reject_plan_mode`, `cancel_plan_mode`, `list_for_bear`, `active_for_session`, `get_*`, `render_plan_artifact_markdown` | C | unchanged |
| module `acp_plan_mode` | C | `plan_mode` |

### 5. `acp_sessions.rs` (51) — **S** (mostly core; mode/resume are wire-tagged)

Conversation↔session mapping over Postgres. Session/conversation identity + persistence is core; `current_mode` (plan mode) and `adapter_environment` reflect a client/wire context but remain stored centrally.

| Current | Bucket | Target |
|---|---|---|
| `AcpSessionRow`, `UpsertAcpSession`, `SessionListParams` | C | `SessionRow`, `UpsertSession`, `SessionListParams` |
| `upsert_session`, `mark_resolved`, `find_for_user_bear_session`, `list_for_user_bear`, `mark_closed`, `mark_archived`, `set_title_for_bear_conversation`, `mark_title_synced`, `update_client_conversation_title`, `resolved_conversation_ids_for_bear` | C | unchanged |
| `set_current_mode`, `update_adapter_environment` (ACP plan-mode + adapter env) | S | keep in core store, but document columns as wire-influenced; setters fed by the adapter |
| module `acp_sessions` | C | `sessions` |

### 6. `acp_events.rs` (125) — **W** (wire projection; reuse existing core seam)

The ACP-flavored event model + its SSE adapter + Letta-named mappers. The core already has `RuntimeSemanticEvent`/`RuntimeStreamEvent` + `bearwire_projection`; this module is the ACP overlay.

| Current | Bucket | Target |
|---|---|---|
| `AcpGatewayEvent` (enum) | W | move to `den-acp`; it is the ACP wire-event type |
| `map_native_letta_stream_event_to_acp_event`, `…_with_accumulator`, `native_letta_conversation_resolved_event` | W | move to adapter projection; **delete the `letta` naming** |
| `acp_event_to_adapter_sse`, `acp_event_adapter_type`, `acp_event_has_visible_output` | W | adapter SSE projection |
| `letta_inner`, `letta_stream_text_preserving_whitespace` | W (or delete) | adapter-internal; rename off `letta` |
| `ToolCallAccumulator` (+ `observe`, buffer accessors) | S | the streamed tool-call delta accumulator is generic stream parsing → keep a **core** accumulator emitting `RuntimeStreamEvent`; the adapter projects to `AcpGatewayEvent` |
| module `acp_events` | W | folded into `den-acp` projection over BearWire |

### 7. `acp_tools.rs` (406) — **S** (largest split: policy=core, client advertisement=wire)

"ACP projection of the `den_core::tools` surface." Two distinct halves:

| Current | Bucket | Target |
|---|---|---|
| `AcpToolClass`, `AcpToolEnablementState`, `AcpResolvedSessionPolicy`, `resolve_session_policy`, `resolve_session_policy_for_mode`, `acp_tool_policy`, `tool_class`, `acp_provider_tool_allowed_in_policy`, `provider_tool_name_is_safe`, `AcpToolStatus` | C | tool **policy/class** is runtime: `ToolClass`, `ResolvedSessionPolicy`, `tool_policy`, … |
| `AcpToolName` enum + all `ACP_*_TOOL` descriptors (`read_text_file`, `list_directory`, `edit_file`, `git_*`, `terminal_run_command`, `process_run`, `chrome_*`, `mcp_call`), `AcpToolDescriptor`, `from_provider_alias`, `descriptor`, `required_string_args` | W | the **client-advertised ACP filesystem/terminal/chrome tool vocabulary** → `den-acp` |
| `acp_client_tool_descriptors`, `…_for_client_context`, `acp_provider_tool_names_for_client_context`, `acp_read_text_file_client_tool_descriptor`, `acp_client_tool_descriptor`, `supported_provider_tool_names` | W | ACP client tool **advertisement** → `den-acp` |
| `acp_tool_display`, `acp_tool_display_for_provider`, `acp_tool_policy_json_for_provider` | S | display **shape** is core (`ToolDisplayDescriptor`, see den-core); the ACP-JSON shaping is wire |
| string constants `LETTA_TOOL_CALL_MAPPED`, `LETTA_CONTINUATION_*` | C | rename off `LETTA` |
| module `acp_tools` | S | split into core `tool_policy` + adapter `acp/client_tools` |

## `den-core` touchpoints

| Current | Bucket | Target |
|---|---|---|
| `tools/display.rs::AcpToolDisplayDescriptor` (re-exported `tools::mod`) | C | `ToolDisplayDescriptor` |
| `DenToolInvocationContext.acp_session_id`; `memory::source_acp_session_id`; `tools/plan_mode::require_acp_session`; `tools/support` & `environment/payloads` & `memory` JSON keys `acp_session_id` | C | concept = external **client session id** (`SessionId` is already neutral in `ids.rs`): rename field/helpers → `client_session_id` / `source_client_session_id` |
| `tools/plan_mode/*` (tool surface + `store` trait taking `acp_session_id`) | C | already core capability; align naming with renamed `session_id` |
| `config.rs::acp_gateway_enabled` (+ `ACP_GATEWAY_ENABLED` env) | W | retired; API listener now mounts ACP compatibility and BearWire routes directly |
| `tools/environment/payloads.rs` `acp`/`acp_variant` adapter-environment reporting | S | the runtime status payload is core; the `acp`-labeled variant is the adapter's environment projection |
| `ids.rs` doc comments referencing `acp-…` session ids | C | comment-only; keep, neutralize wording |

## Cross-cutting (outside the two crates, but blocking the goal)

1. **Neutralize `ApiState`** (`den-acp/src/service.rs`): it carries `acp_tool_turns: AcpToolTurnCoordinator` and `acp_turn_cancellations: AcpActiveTurnCancelRegistry`. Per ADR-0043 principle 3, the shared state becomes a protocol-neutral `DenState` holding only protocol-agnostic deps; the tool-turn / cancel registries become **adapter-owned** (not fields on shared state).
2. **Break the `den-api → den-acp` inversion**: `den-api` depends on `den-acp` only to obtain `ApiState`. Once state is neutral and composition-owned (a state/composition layer below all edges), `den-api`, `den-acp`, and `den-web` become peers — none depends on another.
3. **Adapter consumers** (`den-acp/src/acp/**`, `den-acp/src/core/acp/**`, binary `src/core/*bridge_tests.rs`) reference the renamed core symbols (`den_runtime::acp_tool_turns::*`, `…::acp_turn_controller::*`). They update to the neutral paths; transitional `pub use` shims keep them compiling during the staged migration.

## Summary counts

- **Pure core renames (C):** `acp_turn_controller` → `turn_controller`, `acp_tool_turns` → `tool_turns`, `acp_turn_runner` → `turn_runner`, `acp_plan_mode` → `plan_mode`. (~4 modules, ~60 public items — unambiguous, lowest risk.)
- **Splits (S):** `acp_sessions` (mostly core), `acp_events` (mostly wire), `acp_tools` (policy core / advertisement wire). (~3 modules — the real design work.)
- **`den-core`:** 1 type rename (`AcpToolDisplayDescriptor`), 1 field/helper rename (`acp_session_id` → `client_session_id`), retired edge flag (`acp_gateway_enabled`), comment cleanups.
- **Cross-cutting:** neutralize `ApiState` → `DenState`; relocate two registries to the adapter; remove `den-api → den-acp` dependency.

## Recommended staging (each step keeps the test net green)

0. **Precondition:** add the missing `bearwire_projection/golden_traces_tests.rs` safety net (see below) before any rename.
1. **Pure core renames** — `turn_controller`, `tool_turns`, `turn_runner` (drop `Acp*`), with transitional re-export shims.
2. **`den-core` renames** — `ToolDisplayDescriptor`, `client_session_id`.
3. **`acp_sessions` → `sessions`** — core identity/persistence; tag mode/adapter_env as wire-fed.
4. **`acp_plan_mode` → `plan_mode`** — capability stays core.
5. **Wire moves** — relocate `acp_events` (`AcpGatewayEvent` + projection, de-Letta-named) and the `acp_tools` client-advertisement half into `den-acp`; keep policy/class + accumulator in core emitting `RuntimeStreamEvent`.
6. **Neutralize `ApiState` → `DenState`**; move registries to the adapter; break the `den-api → den-acp` inversion; make edges peers.
7. **Remove transitional shims**; assert zero `acp_*`/`Acp*` in `den-runtime`/`den-core`.

## Safety net (precondition, currently missing)

ADR-0043 and `DEN_RUNTIME_PLAN.md` (Phase 5) reference **`bearwire_projection/golden_traces_tests.rs`** as the harness that asserts `OpenAI SSE → semantic events → BearWire projection → adapter SSE`. **That file does not exist yet.** Present coverage to lean on: `den-acp/src/acp/tests.rs` (98 tests), `den-runtime/.../bearwire_projection/test.rs` (8), and `den/src/core/runtime_bearwire_bridge_tests.rs` (full-pipeline). Before step 1, add the golden-trace file so the rename has a behavior-locking net, as the ADR assumes.

## Implementation outcome

All staged steps landed behind the golden-trace net. Mapping of plan → result:

| Step | Outcome | Commit(s) |
|---|---|---|
| 0. Golden-trace net | `bearwire_projection/golden_traces_tests.rs` added (15 tests) | `fb1f4d6a` |
| 1. Pure core renames | `turn_controller`, `tool_turns`, `turn_runner` (dropped `Acp*`) | `bc8d1571`, `1f838205`, `5c6ad2ef` |
| 2. `den-core` `AcpToolDisplayDescriptor` → `ToolDisplayDescriptor` | done | `087ffd4b` |
| 4. `acp_plan_mode` → `plan_mode` | capability stays core | `38ab2d02` |
| 5a. `acp_events` → `gateway_events` / `AcpGatewayEvent` → `GatewayEvent` | de-Letta-named; canonical model kept in core | `e753ecb2` |
| 5b-A. `acp_tools` → `client_tools` | neutral in-place rename (kept core: it is core tool vocabulary, not just advertisement) | `38cbfa97` |
| 5b-B. Descriptor split | ACP wire methods (`adapter_method`/`client_method`) + descriptor advertisement moved to `den-acp::acp::client_tool_advertisement` (`AcpWire` table); core descriptor + `ToolPolicy::to_json` no longer carry wire methods | `79abe689`, `211a6095` |
| 6. `ApiState` → `DenState` | moved to `den-service`; state inversion fixed | `70c5344a` |
| 6-follow-on. Composition root | `create_api_app` takes injected peer routers; `den-api` no longer depends on `den-acp` | `39ad7a73` |
| 7. Shim sweep + residual doc | this section | — |

### Step 7: shim sweep

**No transitional shims remained to remove.** Each rename was done in place (via `git mv` + reference rewrites), so no back-compat `pub use … as acp_*` aliases were introduced in `den-runtime`/`den-core`. The only `as acp_*` re-exports in the tree are inside the **`den-acp` edge** (`den-acp/src/core/mod.rs`: `pub use acp::{runtime,tokens,turn_runner} as acp_*`), which are appropriate there (that crate *is* the ACP edge) and are consumed by the binary's `core/mod.rs`.

### BearWire armature-obligation invariant

Follow-up tightening (2026-07) clarified a protocol boundary rule for permission-mediated armature work: Den must not emit an armature-actionable wait event unless the exact corresponding BearWire obligation has already been durably persisted and can be answered by the method named in the event. The canonical v1 event shape is `client.waiting` with `data.obligation_id`, `data.expected_client_method`, nested `data.tool_call`, and nested `data.permission`. See `docs/architecture/bearwire-json-spec.md` for the wire shape.

This keeps permission mediation in the BearWire wire layer while preserving a protocol-agnostic runtime core: the core emits semantic tool/permission waits, and the BearWire projection turns them into answerable client obligations.

### Residual `acp_*` / `ACP` surface in Den — strict compatibility rule

Follow-up tightening (2026-06) raised the bar: even DB/log/tool names are candidates for removal unless they have a concrete compatibility reason to remain. The first tightening pass removed more Den-owned vocabulary without changing behavior:

1. `den_protocol::AcpConversationRuntime` → `SessionConversationRuntime` (deprecated alias retained temporarily).
2. `DenState.acp_turn_cancellations` → `turn_cancellations`.
3. In-memory tool-turn / cancellation structs now use `client_session_id` rather than `acp_session_id`; ACP JSON response keys stay in the adapter.
4. Runtime startup capability names moved from `acp_requires_runtime` / `runtime_required_for_acp` to `edge_gateway_requires_runtime` / `runtime_required_for_edge_gateway` (deprecated alias retained temporarily).

Remaining `acp`/`ACP` names require explicit justification, not blanket exemption:

1. **Persisted schema names** — tables/columns such as `acp_sessions`, `acp_session_id`, `acp_plan_mode_sessions`, and `source_acp_session_id` are live SQL schema and persisted data contracts. Removing them requires additive migrations, SQL aliases, backfill/compatibility windows, and query updates. Do not rename in-place.
2. **ACP edge protocol names** — names in `den-acp` are allowed when they describe the ACP adapter/wire contract.
3. **Stable external tool identifiers** — current `client_tools` `canonical_name: "acp.<domain>.<tool>"` values may be persisted in policy JSON/logs. They should be migrated only with an aliasing/resolver plan so old records continue to resolve.
4. **Deploy/env compatibility** — `Config::acp_gateway_enabled` / `ACP_GATEWAY_ENABLED` was retired; API exposure is controlled by `RUN_API`.
5. **Observability strings** — diagnostic values like `"component": "den.acp"` are lower risk but may feed dashboards; change with metric/log compatibility awareness.

**Current result:** Den-owned in-memory state and protocol traits are being neutralized. Remaining names are either ACP-adapter surface or compatibility-bound persisted/deploy/observability identifiers that need explicit migration plans rather than mechanical renames.
