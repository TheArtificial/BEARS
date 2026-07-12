# Bear Den Architecture

A consolidated view of the whole system: what the pieces are, how a turn executes, where state lives, and which boundaries matter. This page summarizes the canonical architecture docs; when detail here disagrees with them, the canonical docs win. Start with [docs/architecture/den-runtime.md](docs/architecture/den-runtime.md) and [docs/architecture/overview.md](docs/architecture/overview.md) for the authoritative versions.

## The one-paragraph version

**Bear Den** hosts durable assistant identities called **Bears**. **Den** (Rust, `services/den/`) is the runtime and control plane: it runs one in-process agent loop for every kind of work, talks directly to **Bifrost** (the OpenAI-compatible model gateway) for inference, and enforces all policy over tools, memory, approvals, and autonomy. A Bear operates through five **stances** — `chat`, `pair`, `curate`, `work`, `watch` — which are capability and trust profiles over that single loop, not separate agents. Bear cognition is canonical in **per-Bear SQLite**; everything control-plane (conversations, approvals, tasks, identity, compiled prompts, schedulers) is canonical in **Den Postgres**. Protocol surfaces — ACP/BearWire armatures, web chat, APIs — are edge adapters that project the same core runtime.

## Vocabulary

These five words carry most of the architecture ([docs/architecture/stance-vocabulary.md](docs/architecture/stance-vocabulary.md), [docs/GLOSSARY.md](docs/GLOSSARY.md)):

| Term | Meaning |
|------|---------|
| **Bear** | The durable assistant identity users interact with. One Bear spans many surfaces and sessions. |
| **Stance** | The primary operating boundary inside a Bear: capability posture, memory scope, tool surface, approval behavior. Code shorthand: `Profile` / `BearProfile`; documentation also says *trust profile*. |
| **Channel** | A conversation surface (web chat, messaging). Not assumed to have trusted access to a user's machine. |
| **Armature** | A trusted work surface that can expose local tools (ACP-enabled editors, local workspace harnesses). A stronger trust contract than a channel. |
| **Work surface** | A durable domain of work — a repository, service, deployment, Docket project, Cabinet Mission, or long-running responsibility. Plans, memory, and tasks attach to work surfaces. |

Avoid "agent" as the primary noun: it blurs the Bear identity, the stance boundary, and the runtime machinery into one word.

## Components

| Component | Responsibility |
|-----------|----------------|
| **Den runtime core** (`services/den/`) | Turn orchestration, context assembly, tool execution, continuation, compaction, semantic event production |
| **Den Postgres** | Conversations and transcript artifacts, approvals and client obligations, users/membership/Bears, compiled prompt configs, prompt-memory blocks, Docket jobs/tasks, reflection scheduler/queue |
| **Per-Bear SQLite** (`memory.sqlite`) | Canonical Bear cognition: memory records, links, promotions, proposals, observations, reflection outcomes |
| **Bifrost** (`services/bifrost/`) | OpenAI-compatible model gateway; Den's only inference substrate |
| **ACP/BearWire edge** (`tools/bear-armature/`, `den-bearwire`) | Trusted armature integration: local tools, permission UI, turn projection |
| **Web/API edges** | Browser chat, operator/admin surfaces, JSON APIs |
| **Sandboxes** (`bears-sandbox-runner`) | Den-managed isolated workspaces for `work` execution: paired workspace + egress-gateway containers |
| **Garage** (`services/garage/`) | Object storage for artifacts |
| **Git** | Human-authored artifacts only: prompt fragments, policies, skills, schemas. Not canonical for any live machine-written memory. |

```text
Humans / clients
  ├─ ACP armatures (editors, local tools)
  ├─ web chat / operator UI
  └─ API consumers / future channels
          │
          ▼
       Den edges (protocol adapters)
          │
          ▼
   Den runtime core  ── one in-process loop for every stance
  ├─ turn orchestration      ├─ approval handling
  ├─ context assembly        └─ semantic event stream
  ├─ tool routing
          │
   ┌──────┴──────────┬───────────────┐
   ▼                 ▼               ▼
Bifrost         Den Postgres   per-Bear SQLite
(inference)     (control plane) (Bear cognition)
```

## One loop, five stances

There is exactly one agent loop, executed in-process inside Den ([ADR-0035](docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)). A turn is a spawned Tokio task owned by Den; cancellation is a `CancellationToken`; at most one turn is active per (bear, stance, channel).

Stances differ only by **capability profile**: tool roster, memory scope, approval/autonomy policy, compiled prompt, and whether they get a sandbox. See [SAFETY.md](SAFETY.md) for the trust model and [docs/architecture/bear-stances.md](docs/architecture/bear-stances.md) for the full stance reference.

| Stance | Job | Surfaces |
|--------|-----|----------|
| `chat` | Synchronous conversation; captures task intent | Web chat, messaging channels |
| `pair` | Live collaboration inside the user's tools | ACP armatures (IDEs and similar) |
| `curate` | Memory integration, review, promotion, skill governance | Internal only |
| `work` | Approved background execution | Docket dispatch, sandboxes |
| `watch` | Inbound event intake and observation | Webhooks, polling, subscriptions |

"Agent patterns" (plan-then-execute, reflect-on-fail, critique, fan-out) are data-driven **strategy policy** over the one loop, not forked runtimes.

## How a turn executes

1. An edge (ACP armature, web chat, Docket dispatch) authenticates the session and resolves the Bear + stance.
2. Den assembles **turn context**: compiled stance prompt, key memory projection, optional derived recall, prompt-memory blocks, runtime supplements, projected transcript, and the merged tool descriptor surface. (Detailed in [MODEL_EXPERIENCE.md](MODEL_EXPERIENCE.md).)
3. Den streams inference through Bifrost.
4. The model emits content and possibly tool calls. **Den-hosted tools** (memory, session, web retrieval, task tools) execute in-process; **armature-local tools** (filesystem, git, terminal) become client obligations awaited over BearWire/ACP; `work` shell/fs tools run in a Den-managed sandbox.
5. Approvals pause the turn on a Den-stored decision and resume the same in-process task. The core turn coordinator — not any edge — decides when continuation is legal ([ADR-0048](docs/decisions/adr-0048-core-turn-client-obligation-coordinator.md)).
6. Den persists replayable transcript artifacts (including tool calls/results as first-class model-history state) and projects user-visible updates to the edge.

Agent loop control (wall-clock, tool-class, failure, ko, checkpoint, control-level, and context-window signals) governs loop health ([ADR-0050](docs/decisions/adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md), [ADR-0047](docs/decisions/adr-0047-context-window-budget-and-token-estimation.md)). Exploration-heavy classes such as `read`/`search` may receive a small fresh verification window after a successful meaningful mutative step, but turn-global safety fuses still do not reset.

## The storage boundary

The most important line in the system is **Bear cognition vs Den control plane** ([den-runtime.md § storage boundary](docs/architecture/den-runtime.md#storage-boundary-bear-cognition-vs-den-control-plane)):

- **Bear cognition → per-Bear SQLite.** What the Bear knows and how it decided to know it: stance-local and shared/promoted memory, proposals, watch observations, promotion audit, reflection-run outcomes. Familiar logical paths (`core/`, `pair/`, `work/`, …) are a projection over append-only records, not a filesystem.
- **Control plane → Den Postgres.** Infrastructure the Bear plugs into: conversations/transcripts, approvals, identity and membership, compiled prompt configs, Docket jobs/tasks, reflection scheduler/queue.

The metaphor: a Bear *uses* Den's trackers and schedulers the way a person uses a project tracker. The tracker is infrastructure, not part of the person. Cross-store references are by id only — there is no content-sync seam.

Two consequences of this boundary:

- **Tasks are not memory.** Docket jobs/tasks ([ADR-0034](docs/decisions/adr-0034-jobs-and-tasks-work-management.md)) are durable work-management state in Postgres, never part of Bear cognition.
- **Bears are portable.** The SQLite side plus configuration is exactly what moves when a Bear is exported to another Den server. See [PORTABILITY.md](PORTABILITY.md).

Semantic retrieval (Qdrant vectors) is a **derived recall index** — rebuildable from canonical sources, never a second source of truth ([ADR-0038](docs/decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)).

## Core is protocol-neutral; edges adapt

The turn controller, tool coordinator, session machinery, and semantic event stream are core organs of the runtime and carry neutral names ([ADR-0043](docs/decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)). ACP, the web UI, and REST are sibling adapters that project canonical BearWire semantic events to and from their wire formats.

- **BearWire** is the Den ↔ armature wire: session/turn lifecycle, semantic runtime events, tool-call projection, client obligations. It projects runtime semantics; it does not own them.
- **Channels vs armatures:** channels carry conversation; armatures additionally expose trusted local tools with permission UI. Channels should not be forced to pretend to be armatures ([docs/architecture/bear-channel-and-acp.md](docs/architecture/bear-channel-and-acp.md)).
- Client sessions are not canonical conversations. Transcript and tool history live in Den conversation persistence; model replay and user-visible history are separate projections over it.

## Work management, reflection, and autonomy

- **Docket** ([ADR-0034](docs/decisions/adr-0034-jobs-and-tasks-work-management.md)) is the durable work system: jobs as orchestration containers, tasks with completion criteria and run-scoped state, acceptance criteria as the definition of done. A job's task tree *is* the plan for background work.
- **Background execution:** a Bear or human creates/approves Docket work; the scheduler dispatches it to `work`, which executes under a constrained capability profile in a sandbox with an egress gateway ([ADR-0037](docs/decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md)).
- **Reflection** ([docs/architecture/reflection-system.md](docs/architecture/reflection-system.md)): scheduled runs read canonical memory, propose and review promotions. The scheduler/queue lives in Postgres; the canonical run record and outcomes live in per-Bear SQLite.
- **Supervision** is a run-scoped **governance mode** (interactive, grace, autonomous continuation, observational, frozen) orthogonal to the stance ([ADR-0039](docs/decisions/adr-0039-trust-profiles-and-governance-modes.md)) — see [SAFETY.md](SAFETY.md).

## Implementation map

The Den Rust workspace decomposes roughly as ([docs/architecture/den-concepts-overview.md](docs/architecture/den-concepts-overview.md), [docs/architecture/den-crate-architecture.md](docs/architecture/den-crate-architecture.md)):

| Area | Responsibility |
|------|----------------|
| `services/den/crates/den-runtime/` | Agent loop, turn execution, context assembly, semantic events |
| `services/den/crates/den-core/` | Core vocabulary, tool descriptors, shared types |
| `services/den/crates/den-service/` | Shared service state and domain services |
| `services/den/crates/den-memory/` | Per-Bear SQLite memory store |
| `services/den/crates/den-docket/` | Jobs/tasks work management |
| `services/den/crates/den-bearwire/` | BearWire edge for armatures |
| `services/den/crates/den-llm/` | Bifrost client and model registry |
| `services/den/src/core/tools/` | Concrete Den-hosted tool executors |
| `tools/bear-armature/` | ACP/BearWire armature adapter: local tool execution, permission projection |

## Historical note

Earlier phases built on Letta (external agent server), Letta Code/Codepool (harness process), and a git-backed MemFS memory sidecar. All of these are removed: the in-process Den loop *is* the runtime. Docs mentioning them ([docs/architecture/den-architecture.md](docs/architecture/den-architecture.md), [docs/archive/letta/](docs/archive/letta/), parts of [docs/website/how.md](docs/website/how.md)) are historical material, not descriptions of the live system.

## Canonical sources

- [docs/architecture/den-runtime.md](docs/architecture/den-runtime.md) — canonical runtime architecture
- [docs/architecture/overview.md](docs/architecture/overview.md) — one-page system picture
- [docs/architecture/README.md](docs/architecture/README.md) — full reading order
- [docs/architecture/den-bear-spec.md](docs/architecture/den-bear-spec.md) — Bear/stance contract
- [docs/architecture/bears-and-den.md](docs/architecture/bears-and-den.md) — product boundary
- [docs/architecture/memory-model.md](docs/architecture/memory-model.md) — memory model
- [docs/decisions/](docs/decisions/README.md) — ADRs (rationale and history)
