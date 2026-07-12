# How Den Builds Bears

Concepts TODO:
- model ops
- channels and armatures
- tools
- internal bear architecture: stances, memory, skills
- memory curation: conversation / bear memory / skills / cabinet

Den does not merely send prompts to a model. It assembles a Bear from several connected concerns: visible behavior, durable identity, current context, memory, tools, collaboration, curation, and runtime infrastructure.

This document is the beginning of a hierarchical explanation. The first and last sections are framing sections; the middle sections are intended to expand into deeper documentation over time.

## 1. Surface: Conversation and Work

This is what people directly experience when they interact with a Bear: conversation, judgment, collaboration, and useful work products. At this level, the Bear appears as a helpful counterpart that can answer, ask, plan, edit, summarize, decide, and carry work forward.

The surface is where the value is felt, but it is not where the Bear is built. The visible experience depends on the deeper systems below.

## 2. Identity: Charter and Role

What gives the Bear a durable sense of purpose.

- **Charter** — the Bear's durable responsibility boundary: what it exists to care about.
- **Role** — the mode of work it is performing: talk, pair, curate, work, watch, etc.
- **Operating style** — tone, habits, collaboration preferences, and expected behavior.
- **Responsibility boundaries** — what the Bear should own, avoid, escalate, or ask about.
- **Human relationship** — who the Bear is serving or collaborating with in a given session.

## 3. Context: Current Situation

What the Bear knows right now.

- **Active request** — the user's immediate goal, question, or task.
- **Session state** — who is present, what conversation is active, and what has happened recently.
- **Workspace state** — files, projects, tools, services, or external systems currently relevant.
- **Task frame** — constraints, deadlines, permissions, assumptions, and success criteria.
- **Relevant surroundings** — selected memory, active Mission, current domain, or open workstream.

## 4. Memory: Continuity

How the Bear carries knowledge forward.

- **Core memory** — canonical shared knowledge the Bear should retain across roles.
- **Role-local memory** — memory specific to talk, pair, curate, work, watch, etc.
- **Domains** — Bear-specific knowledge areas under that Bear.
- **Derived indexes** — searchable semantic views over canonical memory, not the source of truth.
- **Memory lifecycle** — capture, review, promotion, correction, pruning, and archival.

## 5. Action: Tools and Permissions

How the Bear can safely do things.

- **Tool catalog** — model-facing capabilities such as memory search, file edit, web fetch, etc.
- **Permission model** — what the Bear may do automatically, request approval for, or never do.
- **Adapters** — the bridge between Bear-facing tools and underlying systems.
- **Execution loop** — the Bear decides, calls a tool, observes the result, and continues.
- **Safety boundaries** — validation, scoped access, confirmations, and auditability.

## 6. Coordination: Missions and Relationships

How Bears work with people, teams, and each other.

- **Cabinet Missions** — shared work or knowledge containers involving humans and Bears.
- **Bear participation** — Bears can join Missions without owning them.
- **Shared context** — Mission history, artifacts, goals, and constraints.
- **Handoffs** — one Bear can leave useful context for another Bear or future session.
- **Collaboration patterns** — solo assistance, pair work, review, delegation, monitoring.

## 7. Curation: Reflection and Review

How Bears improve without turning memory into a mess.

- **Reflection** — the Bear notices what may be worth remembering or improving.
- **Review requests** — role-local observations can be submitted for curation.
- **Curation** — selected knowledge is cleaned, organized, and promoted into better places.
- **Human override** — people can correct, delete, approve, or reshape what the system believes.
- **Provenance** — important memory and decisions should remain explainable.

## 8. Infrastructure: Den Runtime

This is the substrate that makes the Bear possible. Den authenticates the human, assembles the session, selects relevant context, routes tools, stores durable state, exposes APIs, and observes the runtime.

The infrastructure should usually stay backstage in the marketing story. It matters because it makes the Bear safe, durable, and explainable, but the public explanation should lead with the Bear people experience rather than the machinery that serves it.

## Codebase anchors

This is a working map for keeping the marketing hierarchy connected to the implementation. Some anchors are current Letta-era seams that should narrow or disappear as Den-native runtime work replaces Letta-owned behavior.

| Section | System meaning | Current codebase anchors |
|---|---|---|
| **Surface** | The human-visible conversation and work experience. | Den web/API surfaces in `services/den/src/web/` and `services/den/src/api/`; ACP/pair entrypoints in `services/den/src/api/acp/`; Codepool streaming runtime in `services/codepool/src/bear-channel.ts`, `services/codepool/src/server.ts`, and `services/codepool/src/pool.ts`; Den-owned transcripts in `conversations`, `conversation_messages`, and compaction/archive tables. |
| **Identity** | Bear identity, membership, role runtimes, and responsibility boundary. | Bear registry and membership in `bears` and `user_bear`; role runtime bindings in `bear_agents`; Rust domain code in `services/den/src/core/bears/`; provisioning/runtime-plan code in `services/den/src/core/bears/provision.rs`, `runtime_plan.rs`, and `letta_code_harness.rs`; current role names are `talk`, `pair`, `curate`, `work`, and `watch`. Charter should remain a Bear property/concept, not a separate Charter entity. |
| **Context** | The current situation Den assembles for a turn. | `bears.context_profile`; scoped prompt/context blocks in `prompt_memory_blocks`; client session/workspace binding in session/execution tables; workspace context on `conversations` and Docket execution/session state; runtime context assembly in `den-runtime`; runtime environment exposure through Den tool descriptors and environment payloads. Historical `bear_work_plans` context exists only in migrations/archive docs. |
| **Memory** | Durable Bear knowledge and the lifecycle that moves knowledge between role-local and shared memory. | Canonical Bear memory is MemFS/git-backed via `services/memfs-manager/git_memfs_server.py`, Bear fields such as `memfs_repo_path`/`runtime_plan`, and Den memory tooling in `services/den/src/core/tools/memory_*`, `services/den/src/core/memory_manager_head.rs`, and `services/den/src/core/memory_proposals.rs`; role branches should follow `talk/`, `pair/`, `curate/`, `work/`, `watch/`, with `core/` as canonical shared memory. Letta Archives are currently derived retrieval indexes, not source of truth. |
| **Action** | Tools, permissions, runtime execution, and safe effects. | Descriptor-owned tools in `services/den/src/core/tools/descriptor/`, `constants.rs`, `aliases.rs`, and `den_tools_impl.rs`; individual tool families under `services/den/src/core/tools/`; Codepool tool bridge in `services/codepool/src/den-tools.ts`; runtime loop/session pool in `services/codepool/src/pool.ts`; Den-managed skill and subscription schema in `bear_skills_manifest`, `bear_skill_proposals`, and `bear_subscriptions`. |
| **Coordination** | Collaboration across humans, Bears, work surfaces, handoffs, and Cabinet Missions. | Membership in `user_bear`; logical conversations in `conversations`; durable work coordination in Docket tables (`bear_jobs`, `bear_tasks`, runs, task/run state, and events); Docket-backed task-list projections for session working focus; conceptual Cabinet/Mission guidance in `docs/architecture/bear-charter-and-cabinet-missions.md`. Cabinet Missions are shared containers; use `mission_ref` only for Cabinet Missions, not Bear Domains. |
| **Curation** | Reflection, curation, review, provenance, and human override. | Reflection schema in `bear_reflection_runs`, `bear_reflection_run_items`, and `reflection_conversations`; pair reflection and review queue schema in `pair_reflection_runs` and `bear_memory_proposals`; conductor/domain code in `services/den/src/core/reflection_conductor.rs`, `services/den/src/core/pair_reflection/`, and `services/den/src/core/memory_proposals.rs`; review tools in `services/den/src/core/tools/memory_review.rs`; skill review through `bear_skill_proposals`. |
| **Infrastructure** | The runtime substrate and service boundary. | Root `docker-compose.yaml`; Rust/Axum Den in `services/den/`; TypeScript Codepool harness in `services/codepool/`; Python MemFS Manager in `services/memfs-manager/`; current backing services under `services/letta/`, `services/bifrost/`, and `services/garage/`; schema evolution in `services/den/migrations/`; smoke tests in `tests/smoke/`. |
