# Den string hygiene roadmap

Status: draft  
Date: 2026-07-01

## Goal

Reduce avoidable string parsing and string assembly in Den and armatures by moving control state, identifiers, routing decisions, and validation into typed boundaries.

This is not a full inventory. It is a prioritized cleanup roadmap for classes of mistakes that have already produced bugs or are likely to produce protocol, routing, permission, history, or security failures.

## Guiding principle

Strings are acceptable at external boundaries: HTTP, JSON-RPC, SQL rows, env vars, logs, and user-visible text. Inside Den and armatures, structure should be represented as typed Rust values, enums, descriptors, and explicit message/event fields.

Do not use transcript text, rendered errors, prefixes, substrings, or ad hoc JSON blobs as internal control protocols.

## Priority order

1. Model identity stringiness
2. Protocol data embedded in transcript text
3. Session/conversation/run/tool/approval ID newtypes
4. Structured errors
5. Tool descriptors and typed tool arguments
6. Static SQL and typed query construction
7. Typed path/glob/workspace targets
8. Typed config/env parsing
9. Typed transcript/history projections
10. JSON-RPC method constants and generated clients

## Phase 1: model identity stringiness

### Problem

Den, Bifrost, providers, and UI surfaces use different model identity layers. Bugs happen when a canonical Den handle such as `openai/gpt-5.5` is accidentally sent where a provider model ID such as `gpt-5.5` is required.

### Target shape

- Keep separate types for:
  - `DenModelHandle`
  - `ProviderModelId`
  - `ProviderName`
  - `BifrostGatewayHandle`
- Resolve model identity once through the catalog snapshot or future registry.
- Pass a resolved execution shape downstream rather than re-parsing model strings at call sites.

### Work items

- Audit remaining model `String`/`&str` call paths in `den-llm`, `den-service`, `den-bearwire`, `den-web`, and armatures.
- Replace repeated provider-prefix parsing with registry/snapshot resolution helpers.
- Ensure request builders accept typed provider IDs, not raw handles.
- Add regression tests for canonical-handle vs provider-model-ID separation.

### Done when

- No LLM provider request builder accepts an ambiguous raw model string.
- Model routing/capability decisions use the shared catalog snapshot or registry API.

## Phase 2: protocol data embedded in transcript text

### Problem

Control data encoded as XML, Markdown fences, sentinel text, or JSON blocks inside user/assistant messages is fragile and confuses history, replay, UI rendering, and model context.

### Target shape

- BearWire/ACP wire events and typed message parts carry control state.
- Transcript text remains human/model text, not a hidden protocol.
- Tool calls, permission requests, resource updates, compaction, diagnostics, and errors are explicit events or fields.

### Work items

- Audit Den and armatures for parsing or emitting XML/Markdown/JSON control blocks in message content.
- Replace each control block with a typed BearWire event, ACP message part, or explicit request/response field.
- Document any remaining model-facing text conventions as prompt behavior, not runtime protocol.

### Done when

- Armatures do not parse transcript text to discover tool calls, permissions, resources, run state, or errors.
- Den does not require sentinel text in persisted messages to reconstruct protocol state.

## Phase 3: typed IDs and handles

### Problem

Many unrelated identifiers are plain `String` values. This makes it easy to swap session IDs, conversation IDs, run IDs, tool call IDs, and approval IDs, especially across BearWire, ACP, persistence, and runtime continuation code.

### Target shape

Introduce narrow newtypes for distinct concepts, with parsing/validation at boundaries:

- `ClientSessionId`
- `ConversationId`
- `RuntimeSessionId`
- `BearWireRunId`
- `RequestId`
- `ToolCallId`
- `ApprovalId`
- `ArmatureTokenId` / token subject types where appropriate

### Work items

- Start at BearWire run/session/approval paths, where ID confusion has already caused failures.
- Add `FromStr`, `Display`, serde, and SQL binding support only where needed.
- Keep DB schema text columns initially; type conversion happens at repository/service boundaries.
- Avoid broad mechanical rewrites. Convert one boundary at a time with tests.

### Done when

- BearWire session/run/tool/approval methods no longer pass semantically distinct IDs as interchangeable raw strings internally.
- Type errors prevent accidental use of a conversation ID where a client session ID is required.

## Phase 4: structured errors

### Problem

Rendered error messages have been used for classification, causing misleading UX such as connectivity failures being reported as token validation failures.

### Target shape

- Error kinds are structured enums/codes internally.
- Rendered strings are produced only at user/API edges.
- BearWire and armature errors preserve machine-readable fields such as:
  - `kind`
  - `code`
  - `retryable`
  - `http_status`
  - `request_id`
  - `run_id`
  - `server_version`

### Work items

- Define stable BearWire/armature error kind enums.
- Replace substring matching of error text with typed classification.
- Preserve upstream structured error details through JSON-RPC error `data`.
- Add tests for auth vs authorization vs Den unavailable vs Den server error vs protocol validation.

### Done when

- Armature UX decisions do not inspect rendered error strings.
- Token/auth errors cannot mask 502/503/504 Den availability failures.

## Phase 5: tool descriptors and typed tool arguments

### Problem

Tool routing, permission classes, and execution ownership become fragile when inferred from provider names, prefixes, scattered allowlists, or raw JSON arguments.

### Target shape

- Tool ownership and permission classes are descriptor-owned.
- Tool args deserialize into typed structs at the routing boundary.
- `serde_json::Value` is retained for persistence/audit but does not leak deep into execution logic.

### Work items

- Identify current descriptor/resolver sources for Den-hosted, armature-local, and forwarded MCP tools.
- Remove prefix/substr permission inference where descriptors can provide metadata.
- Add typed argument structs for high-risk and frequently used tools first:
  - filesystem paths/edits
  - terminal/process execution
  - permissions
  - memory writes
- Centralize validation and error rendering for tool arguments.

### Done when

- Permission prompts are generated from typed descriptors and typed args.
- New tools cannot be routed or permissioned without descriptor metadata.

## Phase 6: static SQL and typed query construction

### Problem

Dynamic SQL assembly risks injection, invalid queries, and hidden coupling between string fragments.

### Target shape

- SQL is static and parameterized by default.
- Dynamic identifiers are represented by closed enums and whitelisted mappings.
- Query builders are used only where they preserve parameterization and explicit identifier whitelists.

### Work items

- Audit `format!`, `push_str`, and string interpolation around SQL paths.
- Replace runtime SQL fragments with static queries or typed query builders.
- Add review guidance for migrations and repository functions.

### Done when

- No runtime user/config/model input is concatenated into SQL.
- Any dynamic identifier path has an explicit whitelist and tests.

## Phase 7: typed path, glob, and workspace targets

### Problem

Raw paths and globs are interpreted independently by Den, armatures, permission UX, and tool executors. This can produce vague prompts, inconsistent validation, and unsafe path handling.

### Target shape

- Workspace references use typed values:
  - `WorkspaceRoot`
  - `WorkspaceRelativePath`
  - `GlobPattern`
  - `FileEditTarget`
- Validation is centralized for containment, hidden files, symlinks, absolute paths, and limits.

### Work items

- Start with armature-local filesystem tools and permission prompts.
- Make permission target labels come from validated typed targets, not fallback strings like "the requested target".
- Align Den descriptors with armature-local validation metadata.

### Done when

- Filesystem permission prompts identify validated targets consistently.
- Tool execution cannot reinterpret an unvalidated raw path differently from permission evaluation.

## Phase 8: typed config and env parsing

### Problem

Config/env values parsed repeatedly across runtime code can drift in defaults, validation, and normalization.

### Target shape

- Env/config parsing happens once at startup or test setup.
- Runtime code consumes typed config values such as URLs, durations, booleans, enums, sizes, model handles, and feature modes.

### Work items

- Audit repeated `std::env::var`, manual boolean parsing, URL parsing, and model normalization.
- Move parsing into config structs with clear defaults and validation.
- Keep environment variable names as boundary strings only.

### Done when

- Runtime paths do not parse env vars directly.
- Invalid config fails early with structured diagnostics.

## Phase 9: typed transcript/history projections

### Problem

Message history bugs happen when runtime code reconstructs state by concatenating text or filtering raw role/type strings.

### Target shape

- Canonical persisted messages keep explicit boundaries, sequence, role, visibility, message type, source event/run, and content parts.
- Shared projection helpers produce:
  - model transcript
  - user-visible history
  - armature replay history

### Work items

- Audit model replay and armature reload paths for ad hoc role/type filtering or text assembly.
- Move remaining call sites to shared projection helpers.
- Add tests for error turns, empty assistant turns, tool-only turns, compaction, and reload.

### Done when

- Reload/history/model replay do not reconstruct message boundaries from concatenated text.
- User-visible and model-visible projections are independently tested.

## Phase 10: JSON-RPC method constants and generated clients

### Problem

BearWire JSON-RPC method names are scattered string literals across Den, armature, and tests. This makes retirement, renaming, and coverage checks harder.

### Target shape

- Central method constants or enums represent BearWire methods.
- Client helpers use typed request/response structs.
- Tests reference constants rather than spelling method names repeatedly.

### Work items

- Introduce a `BearWireMethod` enum or constants module shared where practical.
- Convert armature BearWire client helpers first.
- Convert Den dispatch/tests after API shape stabilizes.
- Optionally generate clients from a protocol schema once BearWire v1 is stable.

### Done when

- Adding, retiring, or renaming a BearWire method has one obvious source of truth.
- Legacy method use can be detected by compile errors or focused tests.

## Cross-cutting implementation rules

- Prefer small, boundary-focused changes over broad mechanical rewrites.
- Keep DB schemas stable unless a migration is explicitly justified; Rust newtypes can wrap existing text columns.
- Preserve JSON/log output compatibility unless there is a clear migration path.
- Add regression tests at the boundary that previously failed, not just unit tests for wrappers.
- Do not remove useful raw payload persistence for audit/debugging; prevent raw payloads from becoming business logic inputs.

## Suggested first milestone

A practical first milestone should cover the highest-risk BearWire/armature path:

1. Finish model identity type separation in LLM request construction.
2. Add typed BearWire IDs for session/run/tool/approval flows.
3. Replace armature error string matching with structured error kinds.
4. Add typed permission/tool argument structs for filesystem permission prompts.
5. Add tests for a permission-blocked run, tool result continuation, Den unavailable, and session reload after an error.

This milestone directly addresses the recent GPT-5.5 routing issue, permission-obligation failures, misleading token/connectivity errors, and ACP reload/history confusion.
