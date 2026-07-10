# Model experience audit and refresh plan

Status: proposed  
Date: 2026-07-10  
Related:

- [MODEL_EXPERIENCE.md](../../MODEL_EXPERIENCE.md)
- [Non-blocking structured updates](../architecture/non-blocking-structured-updates.md)
- [BearWire JSON specification](../architecture/bearwire-json-spec.md)
- [ADR-0017: provider-safe tool naming](../decisions/adr-0017-provider-safe-tool-naming.md)
- [ADR-0025: tool naming and execution strategy](../decisions/adr-0025-tool-naming-and-execution-strategy.md)
- [ADR-0048: core turn/client-obligation coordinator](../decisions/adr-0048-core-turn-client-obligation-coordinator.md)

## Goal

Make Den's model-facing experience coherent, legible, and resilient across model sizes and surfaces. A model should see a stable, concrete action vocabulary; Den should own runtime semantics, execution location, replay policy, progressive disclosure, and continuation legality through descriptors and typed boundaries.

This refresh should make future tool/action additions naturally follow the same rules instead of depending on contributor memory or scattered string matches.

## Scope

In scope:

- model-facing tool/action naming;
- internal canonical names and aliases;
- descriptor metadata for runtime semantics;
- non-blocking structured update implementation path;
- progressive disclosure of tools/actions;
- prompt/model-facing descriptions;
- BearWire and armature projection consistency;
- tests, fixtures, and contributor gates.

Out of scope for this plan:

- redesigning Docket itself;
- changing provider APIs;
- replacing JSON/JSON-RPC wire encoding;
- broad UX redesign of ACP clients.

## Target model

### Action categories

Every model-facing action must have descriptor-owned semantics:

```rust
enum ModelActionSemantics {
    BlockingTool,
    ClientObligation,
    NonBlockingUpdate,
    EphemeralProgress,
}
```

| Semantics | Blocks model continuation? | Result model-visible? | Examples |
| --- | ---: | ---: | --- |
| `BlockingTool` | Yes | Yes | `fs_read_text_file`, `git_diff`, `web_fetch` |
| `ClientObligation` | Yes | Yes/decision | approval, local write/edit, human input |
| `NonBlockingUpdate` | No | Usually no | `set_conversation_title`, advisory `update_task_status` |
| `EphemeralProgress` | No | No | `report_progress`, phase/status updates |

The model-facing vocabulary can remain function-like and concrete. The runtime category is not exposed as model prompt jargon.

### Naming layers

Use four explicit names for each action/tool, each with one owner:

| Layer | Example | Owner | Purpose |
| --- | --- | --- | --- |
| Model-facing provider name | `fs_read_text_file`, `update_task_status` | descriptor | What the model sees/calls |
| Canonical internal name | `acp.fs.read_text_file`, `den.task.update_status` | descriptor | Stable audit/routing identity |
| Adapter/client method | `fs/read_text_file`, `client.tool.result` | protocol adapter | Wire/client invocation |
| UI label | `Read file`, `Update task status` | descriptor | Human projection |

Rules:

- Provider names are concise action names, snake_case where provider constraints require it.
- Canonical names are dotted and internal only.
- Do not advertise dotted canonical names to the model unless a provider or protocol explicitly requires them.
- Legacy aliases may be accepted at routing boundaries but must not be advertised.
- Routing, permissions, execution target, and replay policy must resolve from descriptors, not string matching.

### Model-facing communication format

Primary model → harness communication is provider-native function/tool calling.

If a provider path lacks native tool calling, the only accepted text fallback is a strict function-call DSL:

```text
report_progress(summary="Found likely config mismatch")
set_conversation_title(title="Fix Coolify sandbox roots")
update_task_status(status="in_progress", summary="Reading deployment config")
```

Do not accept Markdown fences, XML blocks, arbitrary JSON blobs, or protocol event names as control input from the model.

### Progressive disclosure

Progressive disclosure should reduce cognitive load without teaching false per-turn capabilities.

Principles:

1. The stable base surface for a stance/surface remains consistent across turns.
2. Tool groups may be discoverable/loadable through explicit descriptor-owned mechanisms, not prompt heuristics.
3. Defer rare or specialized actions behind catalog/search/load actions where model/provider support permits.
4. Smaller or low-skill models receive simpler action surfaces with fewer status/gating choices.
5. Progressive disclosure must preserve auditability: a loaded/deferred action still resolves to the same canonical descriptor.

## Current risk areas

1. **Semantic overload of tools**
   - `set_conversation_title` and task/status updates can behave like blocking model-visible tools even when they are metadata updates.

2. **Naming drift**
   - Provider-safe snake_case names, legacy ACP-inspired names, and dotted canonical names coexist.
   - Some code paths may still route or classify by string matches rather than descriptors.

3. **Scattered policy**
   - Execution target, approval policy, replay policy, and display behavior have improved but need a single descriptor contract and enforcement tests.

4. **Tool surface size**
   - Large merged Den + client tool surfaces increase token cost and selection ambiguity.
   - Smaller models may misuse broad surfaces or subtle status actions.

5. **Prompt/action mismatch**
   - Model-facing descriptions may not clearly distinguish data-needed tools from advisory progress/status actions.

6. **Replay/projection mismatch**
   - Non-blocking updates need consistent live, replay, and session-load projection without becoming transcript text.

## Workstreams

### A. Descriptor schema refresh

Add or formalize descriptor fields:

```rust
struct ActionDescriptor {
    provider_name: ProviderToolName,
    canonical_name: CanonicalToolName,
    aliases: Vec<ProviderToolName>,
    ui_label: String,
    description: String,
    execution_target: ExecutionTarget,
    action_semantics: ModelActionSemantics,
    approval_policy: ApprovalPolicy,
    replay_policy: ReplayPolicy,
    model_visibility: ModelVisibility,
    progressive_disclosure: DisclosurePolicy,
    risk_class: RiskClass,
}
```

Deliverables:

- descriptor type additions in the narrowest crate that owns tool/action metadata;
- typed enums for action semantics, model visibility, replay policy, and disclosure policy;
- descriptor resolver APIs that answer:
  - provider name → descriptor;
  - alias → descriptor;
  - canonical name → descriptor;
  - stance/surface/model profile → advertised action set.

Acceptance criteria:

- No runtime path decides execution target, blocking behavior, or model visibility from raw tool-name string matches except inside descriptor resolution or narrow compatibility boundaries.
- Tests assert every advertised action has all required descriptor fields.

### B. Naming consistency audit

Inventory all action/tool names:

- provider names advertised to models;
- canonical dotted internal names;
- legacy aliases;
- ACP/client method names;
- UI labels;
- permission classes;
- adapter method names.

Produce a table:

```text
provider_name | canonical_name | aliases | execution_target | semantics | advertised? | owner crate | notes
```

Deliverables:

- generated or checked inventory fixture;
- lints/tests for duplicate provider names and unowned aliases;
- deprecation list for legacy model-facing names.

Acceptance criteria:

- No dotted internal names are advertised to models by default.
- Legacy aliases are accepted only by descriptor-owned alias resolution.
- Provider names follow a consistent concise snake_case style.

### C. Non-blocking structured update implementation

Start with low-risk metadata actions:

1. `set_conversation_title`
2. `report_progress`
3. advisory `update_task_status` / `work.progress.updated`

Implementation direction:

- Keep model-facing action names concrete.
- Mark these descriptors as `NonBlockingUpdate` or `EphemeralProgress`.
- Project successful updates as typed BearWire events such as:
  - `session.metadata.updated`
  - `work.progress.updated`
  - `task.status.updated`
- Do not create client obligations or force model continuation solely because of these updates.
- If provider APIs require a tool result for a tool-shaped call, return a minimal provider-level acknowledgement while preserving Den semantics as non-blocking.

Acceptance criteria:

- A title update can live-update/replay ACP session title without requiring a model-visible tool result for semantic correctness.
- Advisory task progress can update UI/Docket projection without triggering continuation nudges.
- Terminal/gated task changes still require validation, handoff, or blocking coordinator semantics.

### D. Progressive disclosure

Define disclosure policies:

```rust
enum DisclosurePolicy {
    AlwaysAdvertise,
    StanceDefault,
    SurfaceCapability,
    SearchLoadable,
    HiddenInternal,
}
```

Phases:

1. Categorize current tools/actions by disclosure policy.
2. Keep core pair/work actions stable by stance.
3. Defer specialized or rarely used tools behind discovery/load affordances where provider support permits.
4. Add model-profile variants for small/cheap models:
   - fewer update actions;
   - fewer terminal state transitions;
   - more advisory-only progress verbs.

Acceptance criteria:

- Tool availability changes only from descriptor policy, stance, surface capabilities, model profile, or explicit load action.
- No prompt heuristic hides filesystem/git/terminal tools mid-session for ACP armatures.
- Smaller model profiles have explicit, testable action rosters.

### E. Model-facing prompt and schema refresh

For each action descriptor:

- description says what the action does;
- description says when not to use it;
- schema is narrow and enum-backed;
- response/result is high-signal and bounded;
- examples are added only for complex actions and counted against prompt budget.

Guidelines:

- Prefer concrete verbs: `report_progress`, `update_task_status`, `request_handoff`.
- Avoid abstract protocol terms: `emit_metadata`, `surface_update`.
- Avoid broad `metadata`/`extra` bags.
- Use named params in text fallback.
- Keep terminal transitions gated or advisory for weaker models.

Acceptance criteria:

- Descriptor tests enforce required descriptions and schemas.
- Evals/golden prompts cover weaker-model usage of progress/title/task update actions.

### F. BearWire/ACP projection and replay

Update projection rules:

- blocking tool exchanges → tool cards and model replay;
- non-blocking structured updates → metadata/progress/task UI updates and replay according to replay policy;
- ephemeral progress → live status, not durable transcript;
- client obligations → permission/tool wait UI, never inferred from text.

Acceptance criteria:

- Live and replay/session-load projections agree for `session.metadata.updated`, task/work progress, and blocking tool cards.
- Non-blocking updates never replay as assistant answer text.
- ACP clients can list modified files/follow agent independently of whether an update was a blocking tool or metadata event.

### G. Validation and guardrails

Add tests/gates:

- descriptor completeness test;
- provider/canonical alias uniqueness test;
- advertised tool surface snapshot by stance/surface/model profile;
- no model-advertised dotted names test;
- non-blocking update does not create client obligation test;
- non-blocking update does not trigger continuation by itself test;
- replay fixture for title/status update;
- parser tests for fallback function-call DSL if/when implemented;
- grep/lint for raw string match routing outside descriptor resolver modules.

## Phased rollout

### Phase 0 — Audit and inventory

- Generate action/tool inventory table.
- Identify all string-match routing/classification sites.
- Identify all model-advertised names and aliases.
- Classify each action by target semantics.

Exit criteria:

- Inventory checked into docs or fixtures.
- Open questions listed for ambiguous actions.

### Phase 1 — Descriptor semantics foundation

- Add typed descriptor fields for action semantics, model visibility, replay policy, and disclosure policy.
- Backfill current descriptors.
- Add completeness/uniqueness tests.

Exit criteria:

- Every advertised action has explicit semantics.
- Existing behavior is unchanged except for stronger validation.

### Phase 2 — Naming cleanup

- Normalize provider-facing names.
- Hide dotted canonical names from model surfaces.
- Centralize legacy alias handling.
- Update docs and snapshots.

Exit criteria:

- Provider-name snapshot is stable and concise.
- Dotted names remain internal/audit names.

### Phase 3 — Non-blocking metadata updates

- Implement `session.metadata.updated` projection.
- Convert or dual-project `set_conversation_title` into non-blocking update semantics.
- Add replay/session-load coverage.

Exit criteria:

- Title update no longer behaves as a model-dependent tool result in Den semantics.
- ACP title live/list/load/replay remains consistent.

### Phase 4 — Work/task progress updates

- Add `report_progress` / `work.progress.updated` / advisory `task.status.updated` path.
- Coalesce/rate-limit noisy updates.
- Keep terminal task transitions gated.

Exit criteria:

- In-flight status can update without continuation nudges.
- Completion/handoff/resource waits still use coordinator obligations.

### Phase 5 — Progressive disclosure

- Add disclosure policies to descriptors.
- Define base rosters for pair/work/chat and small-model profiles.
- Add optional discovery/load path for specialized tools if provider support is adequate.

Exit criteria:

- Tool/action rosters are generated from descriptors and snapshot-tested.
- Smaller model profiles have simpler tested surfaces.

### Phase 6 — Prompt/eval refresh

- Refresh model-facing descriptions.
- Add focused evals/golden traces for:
  - title updates;
  - progress reporting;
  - task status advisory updates;
  - blocking tool use;
  - handoff/approval gating.

Exit criteria:

- Models use update actions correctly in representative tasks.
- Smaller model profile avoids terminal-state misuse in tests/evals.

## Open design questions

1. Should non-blocking update actions remain provider-native tools forever, or should Den eventually generate some updates out-of-band after a turn?
2. How much state should advisory task updates write directly to Docket versus a separate work-progress stream?
3. What provider paths lack native tool/function calling and need the fallback DSL?
4. Should `report_progress` be exposed to all stances or only work/pair?
5. Which terminal task changes require human validation, runtime validation, or model self-report only?
6. How should progressive disclosure interact with prompt caching and provider tool-search features?

## Immediate next steps

1. Build the descriptor inventory table.
2. Add action semantics fields with backfilled defaults.
3. Add tests that fail if a new action lacks semantics, model visibility, replay policy, or disclosure policy.
4. Prototype `session.metadata.updated` for `set_conversation_title` while keeping compatibility with provider tool-call requirements.
