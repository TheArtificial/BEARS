# Work surfaces and conversations

How **durable work resources** relate to **interactive sessions**, and why product copy should prefer *“start a conversation with this repository”* over *“check out this repo and work on it”* inside an open chat.

**Related:** [ADR-0006 — Bear work surfaces](../decisions/adr-0006-bear-work-surfaces.md), [`bear-roles.md`](../architecture/bear-roles.md), [`interactive-profiles-and-role-axes.md`](../architecture/interactive-profiles-and-role-axes.md), [work-surface resolution plan](../roadmap/WORK_SURFACE_RESOLUTION_IMPLEMENTATION_PLAN.md)

## Two layers (do not collapse them)

| Layer | What it is | Lifetime | Example |
|-------|------------|----------|---------|
| **Work surface** | Bear-level durable resource the Bear can act on | Survives conversations | `BEARS monorepo`, `Production Den deploy`, Cabinet Mission |
| **Conversation / thread** | Interactive session with transcript and turn state | Scoped to a channel session | ACP thread, web chat thread, mobile session |

A work surface is **not** role-owned and **not** an authorization boundary by itself ([ADR-0006](../decisions/adr-0006-bear-work-surfaces.md)). Roles (`chat`, `pair`, …) describe **trust and armature**; the surface describes **what resource** the Bear is reasoning about or manipulating.

## Active surface binding belongs on the conversation

For **execution armature** (anything that will touch files, sandboxes, checkouts, or scoped retrieval), the **primary work surface should be bound to the conversation**, not inferred turn-by-turn from casual language.

Den models resolution roughly as:

| State | Meaning |
|-------|---------|
| `unresolved` | No trustworthy surface for this thread yet |
| `candidate` | Hints exist (workspace root, git remote, user mention) |
| `resolved` | Evidence identifies a surface |
| `confirmed` | User or client explicitly anchored this thread to a surface |

**Confirmed binding is thread-scoped.** It tells Den and the agent: *this conversation operates on that resource* — before the first model turn that might run tools or a sandbox.

That binding is separate from **creating** a surface. Usually you **select or scaffold** an existing Bear work surface (`core/work_surfaces/<slug>/…`) and attach the thread to it.

## Product language: conversation with a resource

Prefer:

> **“Start a new conversation with this repository.”**

Over:

> **“Check out this repository and start working on it.”** (inside an already-open generic chat)

Why:

1. **Substrate clarity** — Checkout, sandbox cwd, and surface-first memory retrieval assume a stable `(conversation ↔ primary work_surface)` link ([environment affordance ADR-0028](../decisions/adr-0028-environment-affordance-and-resource-boundaries.md)).
2. **Honest UX** — The user enters a **scoped working context**, not a one-off command buried in prose.
3. **Reuse of durable memory** — The Bear already knows the repo as a work surface; the conversation is a new **binding**, not a new Bear or new resource entity.
4. **Fewer mid-thread mistakes** — Broad chat that drifts across repos/services is a common failure mode when surface is optional every turn.

The Bear **remembers resources**; humans **open conversations to work on one of them** when armature is involved.

## How this interacts with `chat` vs `pair`

See [`interactive-profiles-and-role-axes.md`](../architecture/interactive-profiles-and-role-axes.md) for the full split. Short version:

| Profile | Human present? | Work surface on thread | Typical UX |
|---------|----------------|------------------------|------------|
| **`chat`** | Yes | Optional; **reference** mode | General conversation; surface hints OK; no co-located execution armature |
| **`pair`** | Yes | Should declare **primary surface early**; **active** mode | IDE, mobile+server sandbox, any UI where the Bear **operates on** the resource |

**`pair` is not “ACP only.”** It means human-present **plus workspace armature** (client tools today; Den sandbox tomorrow). A mobile app that manipulates a repo in a server sandbox is **pair-shaped**, even with a new channel adapter.

**`chat`** remains appropriate when the user is not entering an execution context — Q&A, priorities, task intents, references to a project by name without checkout/sandbox work.

### Trust profile vs governance mode

`chat`/`pair`/`curate`/`work`/`watch` are **trust profiles** (`Profile` in code) — durable trust-and-memory contracts. *How a run is supervised right now* is a separate **governance mode** (`Mode` in code) on the run / workspace session: `interactive`, `grace`, `autonomous_continuation`, `observational`, `frozen`. When a remote `pair` session loses its client, the run transitions `interactive → grace → autonomous_continuation` on the **same** work surface and workspace session — it does not flip the trust profile from `pair` to `work`. See [ADR-0039 — Trust profiles and governance modes](../decisions/adr-0039-trust-profiles-and-governance-modes.md).

## Rules of thumb for builders

1. **New execution context → new conversation bound to a surface** (or explicit “switch work surface” flow), not an implicit re-point mid-thread.
2. **Do not create a new work surface entity for every conversation** unless the resource is genuinely new; bind to an existing slug when possible.
3. **Surface switch mid-thread** should be explicit (like changing project in an IDE), with re-resolution — not “also checkout X” in passing.
4. **Autonomous `work`** carries surface on the **task/run** (Docket), often without a human conversation at all.
5. **`session_info` / resolution tools** expose hints; **confirmed** binding is what execution armature should trust.

## Example flows

### Mobile app + server sandbox (pair-shaped)

1. User picks **BEARS monorepo** (existing work surface) or creates scaffold once.
2. App starts **new conversation** with `work_surface_id` / confirmed slug in session metadata.
3. Den binds sandbox workspace to `(bear, session, work_surface)`.
4. Agent loop runs under **`pair` profile** with active surface mode and scoped tools.

### Slack / web chat (chat-shaped)

1. User asks about deployment process — **no** primary surface required.
2. Optional reference: “in the Den repo we use …” using surface hints or memory search.
3. If user asks for **autonomous** recurring work → task intent → `curate` / `work`, not inline checkout in chat.

### Explicit surface switch (pair or chat)

1. User: “Switch this thread to the Codepool service surface.”
2. Client or Den updates thread binding → `confirmed` new surface.
3. Transcript may note the switch; sandbox/checkout rebinding follows policy.

## Where implementation lives

- Work surface model and anchors: [ADR-0006](../decisions/adr-0006-bear-work-surfaces.md)
- Resolution states and UX copy: [WORK_SURFACE_RESOLUTION_IMPLEMENTATION_PLAN.md](../roadmap/WORK_SURFACE_RESOLUTION_IMPLEMENTATION_PLAN.md)
- `session_info` work-surface hints: Den `work_surface` tool module; active vs reference mode by role
- Role vocabulary: [`bear-roles.md`](../architecture/bear-roles.md)
