# Den-BearWire-Armature-ACP Hardening Plan

**Status:** Proposed  
**Date:** 2026-07-04  
**Related:** [BearWire armature wire implementation plan](BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md), [BearWire turn coordinator refactor plan](BEARWIRE_TURN_COORDINATOR_REFACTOR_PLAN.md), [ACP adapter improvement plan](ACP_ADAPTER_IMPROVEMENT_PLAN.md), [ACP lifecycle reset plan](ACP_LIFECYCLE_RESET_PLAN.md)

## Purpose

Harden the active **Den ↔ BearWire ↔ armature ↔ ACP** path without drifting into architecture-for-architecture’s-sake.

This plan focuses on practical failure modes observed in production-like use:

- tool results that render differently in UI vs model continuation;
- workspace/session context that is present in the armature but lost in Den;
- transcript replay surfaces that drift from one another;
- local filesystem/tool payloads that are still too stringly or too JSON-shaped at important boundaries.

The goal is not to replace BearWire or ACP. The goal is to make the current stack more deterministic, more typed, and easier to debug.

## Scope

In scope:

- BearWire RPC request/response boundaries
- armature local tool execution and result posting
- canonical conversation persistence for tool requests/results
- transcript replay, runtime history replay, and user-history projection
- trusted session workspace context handoff

Out of scope:

- replacing BearWire with another protocol
- redesigning all transcript persistence into new SQL tables
- fully typing every possible adapter-owned extension envelope up front
- introducing a generalized event-sourcing framework

## Guiding Rules

1. Prefer the smallest typed boundary that eliminates a real bug class.
2. Keep open-ended adapter/tool envelopes raw only when extensibility is real and intentional.
3. Centralize normalization once; do not repeat it at every surface.
4. Every transcript-affecting change should be covered at the persistence, replay, and user-history levels.
5. If two surfaces can disagree, add a regression that makes the disagreement impossible to miss.

## Workstreams

### 1. Explicit local tool success/error contract

**Problem:** Local tool behavior is currently spread across conventions for `content`, `structured_content`, `error`, `output_summary`, and `output_preview`.

**Goal:** Define one small shared contract for local tool outcomes that covers:

- success with text
- success with structured payload
- failure with structured error
- incomplete / cancelled / timeout

**Implementation direction:**

- keep a single typed tool-result input model at compaction/persistence seams;
- ensure armature, BearWire, and native runtime all route through it;
- forbid ad hoc empty-string handling at downstream consumers.

**Exit gate:** No active local-tool path hand-builds model-facing tool-result payloads from loose JSON pieces.

### 2. Typed persisted transcript payloads for tool rows

**Problem:** Persisted `content_json` for `tool_request` / `tool_result` still relies on string keys even when the runtime logic is otherwise typed.

**Goal:** Add typed serde-backed payload structs for persisted tool transcript rows.

**Implementation direction:**

- define `PersistedToolRequestPayload` and `PersistedToolResultPayload`;
- decode/encode transcript tool payloads through those structs;
- move replay/user-history/runtime-history helpers off raw key lookups.

**Exit gate:** Tool transcript replay and projection code no longer reaches directly into `content_json["..."]` for active request/result fields.

### 3. Trusted session workspace-context object

**Problem:** `cwd`, `workspace_roots`, and related session context currently cross multiple layers with partial overlap and inconsistent fallbacks.

**Goal:** Use one typed trusted workspace/session context shape across the armature, persisted session row, live runtime session, and `session_info`.

**Minimum fields:**

- `cwd`
- `workspace_roots`
- `source`
- optional adapter snapshot metadata/version

**Implementation direction:**

- parse once at the armature boundary;
- persist as trusted session metadata in Den;
- thread into native runtime session state and tool contexts;
- make `session_info` consume that object directly.

**Exit gate:** ACP Pair `session_info` and runtime context cannot lose workspace roots if the armature provided them.

### 4. End-to-end invariants tests across layers

**Problem:** Many bugs only appear when several layers compose: armature execution, BearWire RPC, Den persistence, replay, and user history.

**Goal:** Add a very small set of high-value end-to-end invariants tests.

**Target flows:**

- local file read success
- local file read missing-path failure
- tool result replay into next-turn model context
- tool result user-history summary

**Implementation direction:**

- keep only 1-2 representative tools;
- assert request decode, persistence shape, replay projection, and user-history projection together;
- prefer integration tests that touch the real active path.

**Exit gate:** A regression in any layer of the active local-tool path breaks a focused typed-flow test.

### 5. Empty-string normalization at important boundaries

**Problem:** Empty strings have repeatedly behaved like valid payloads even when they semantically mean “missing”.

**Goal:** Normalize empty strings to `None` for optional fields unless empty is explicitly meaningful.

**Priority fields:**

- tool result content
- `cwd`
- optional conversation ids / model selection fields
- optional user-history summary text

**Implementation direction:**

- normalize in typed constructors/deserializers;
- do not repeat empty-string heuristics in multiple surfaces.

**Exit gate:** Active persistence/replay/workspace-context code paths do not branch on raw empty-string payloads.

### 6. Centralized status-domain conversions

**Problem:** The stack still has several adjacent status domains with ad hoc conversions.

**Examples:**

- model/persistence tool-result status
- runtime continuation status
- UI finish status

**Goal:** Make conversions explicit and tested.

**Implementation direction:**

- keep one small conversion layer using `From` / `TryFrom` where possible;
- reject unsupported statuses early at the boundary;
- avoid raw string `match` ladders spread across handlers.

**Exit gate:** Status conversion logic lives in one place per domain crossing and is covered by unit tests.

### 7. Stronger armature-side ACP response validation for filesystem tools

**Problem:** The armature currently trusts ACP client responses more than it should for some local filesystem tools.

**Goal:** Make filesystem tool responses deterministic and locally validated where feasible.

**Examples:**

- empty `read_text_file` success must correspond to a real zero-byte file
- stat/list/find results should match basic path/type invariants
- impossible or contradictory combinations should fail early

**Implementation direction:**

- validate path identity and basic shape at the adapter boundary;
- use local filesystem checks where they are deterministic and cheap;
- avoid inferring correctness from human-oriented error text.

**Exit gate:** Missing-file and similar filesystem tool failures cannot masquerade as successful empty results.

### 8. Lightweight live-session diagnostics surface

**Problem:** When session scope is wrong, debugging requires too much inference across armature, Den, and runtime state.

**Goal:** Provide one trusted debug payload for current live session context.

**Suggested fields:**

- stored session `cwd`
- stored `workspace_roots`
- adapter environment snapshot keys
- active runtime session workspace roots
- whether a native continuation session is live

**Implementation direction:**

- expose through an internal operator/debug surface or a clearly non-model-facing RPC method;
- keep it typed and minimal.

**Exit gate:** “Why is session_info wrong?” can be answered from one diagnostics snapshot.

### 9. Reduce duplicated path/permission logic

**Problem:** Path resolution, workspace-root containment, and local-tool permission decisions still appear in multiple places.

**Goal:** Make one canonical validation path per tool family.

**Implementation direction:**

- centralize path resolution + containment checks for filesystem tools;
- centralize permission-scope derivation for local tool requests;
- keep callers thin.

**Exit gate:** Filesystem tool families do not maintain subtly different path-validation behavior across call paths.

### 10. Development/test strict mode

**Problem:** Some malformed inputs are currently tolerated until they cause downstream behavioral drift.

**Goal:** Add a strict development/test mode that rejects malformed or incomplete typed envelopes early.

**Examples:**

- unsupported tool-result status
- missing required typed fields
- invalid empty-required strings
- malformed persisted tool transcript payloads

**Implementation direction:**

- enable in unit/integration tests first;
- keep production tolerant only where backwards compatibility requires it.

**Exit gate:** Common malformed-payload regressions are caught before product-level behavior drifts.

## Delivery Order

Recommended order:

1. explicit local tool result contract
2. typed persisted transcript payloads
3. trusted workspace/session context object
4. end-to-end invariants tests
5. empty-string normalization
6. centralized status conversions
7. filesystem response validation
8. live-session diagnostics surface
9. duplicated path/permission cleanup
10. strict dev/test mode

## Testing Expectations

Every transcript/tooling change in this area should verify, at minimum:

- persistence shape
- model replay projection
- runtime history projection when applicable
- user-history projection when applicable

For tool paths, strongly prefer one focused happy-path and one focused failure-path test.

## Notes

- Open-ended adapter-owned envelopes may remain raw `serde_json::Value`, but that must be explicit in type definitions and comments.
- This plan intentionally stays pragmatic: the target is fewer ambiguity-driven bugs in the active Den/BearWire/armature/ACP path, not a full protocol redesign.
