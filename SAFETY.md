# Keeping Bears Safe: Stances, Trust, and Governance

How Bear Den keeps a powerful, memory-bearing, tool-using assistant safe — primarily through **stances**, the structural trust boundaries inside every Bear. This page consolidates the safety model; canonical sources are listed at the end.

## The threat model in one sentence

An assistant that combines **exposure to untrusted input** (chat messages, webhooks, web content), **the ability to act on the outside world**, and **unrestricted durable memory** in one place can be steered by anyone who can reach its inputs. Bear Den's answer is structural: no single stance ever holds all three powers, and the boundaries are enforced by Den policy, not by prompt guidance alone.

## The five stances as trust boundaries

A Bear feels like one assistant, but internally operates through five stances — durable trust-and-memory contracts, each with a distinct job ([docs/architecture/bear-stances.md](docs/architecture/bear-stances.md)):

| Stance | Raw/private context it sees | External communication | Durable memory writes |
|--------|-----------------------------|------------------------|-----------------------|
| `chat` | Chat/channel context | Conversation only | Own `chat/` branch |
| `pair` | Client/session context (editor, workspace) | Client-mediated, user-approved | Own `pair/` branch |
| `curate` | Broad Bear context (reads across branches) | **None** | Own branch **and** shared `core/` |
| `work` | **Reviewed/curated context only** | Outbound, within approved task scope | Own `work/` branch |
| `watch` | Inbound payloads | **Inbound only** | Own `watch/` branch |

Read the table by columns and the design falls out:

- The stances exposed to raw untrusted input (`chat`, `pair`, `watch`) **cannot take autonomous outbound action** and **cannot write shared memory**.
- The stance that can act outward (`work`) **never sees raw channel history** — only curated context and its approved task definition. A prompt injection arriving via chat, an IDE, or a webhook has no direct path into an outbound action.
- The stance that can write shared memory (`curate`) **has no outbound communication tools at all**.

This is the "Rule of Two" / lethal-trifecta split the [glossary](docs/GLOSSARY.md) refers to: any given stance combines at most two of {untrusted input, outbound action, durable shared state}.

## The flow from raw input to approved action

Nothing moves from an untrusted surface to an external effect without passing through review:

1. A person talks with `chat` or works with `pair`; `watch` independently receives external events and records structured **observations**.
2. Requests implying external or autonomous work become structured **task intents** — not immediate actions.
3. `curate` reviews memories, task intents, observations, skill proposals, and work results. It is the semantic authority; **Den enforces and installs** its decisions.
4. Den dispatches approved tasks to `work`, which executes within the approved scope and writes auditable results.
5. `curate` reviews the results and promotes durable learnings into shared `core/` memory.

The same proposal-and-review pattern governs **skills**: any stance may propose a durable capability change; `curate` approves and chooses which stances it applies to; Den updates the manifest.

## Memory walls

Memory boundaries are the load-bearing part of the stance model ([docs/architecture/memory-model.md](docs/architecture/memory-model.md)):

- Each stance writes only to its own branch (`chat/`, `pair/`, `work/`, `watch/`, curation's own branch). Shared `core/` is written only through `curate`'s promotion flow.
- `work` must never read raw `pair/` or `chat/` — even when a human is inspecting the run. The intended path for `pair` learnings to reach `work` is: `pair/` → review request → `curate` → `core/` or task context.
- Cross-stance sharing flows through reflection and curation, never through raw transcript leakage.
- Transcript history is not memory; unreviewed proposals and observations are never proactively projected into prompts (see [MODEL_EXPERIENCE.md](MODEL_EXPERIENCE.md)).

## Stance is not supervision: governance

A second, orthogonal axis handles "who is watching right now" ([ADR-0039](docs/decisions/adr-0039-trust-profiles-and-governance.md)). The **trust profile** (stance) is the slow-changing trust contract; the **governance** is the fast-changing, run-scoped supervision dial:

| Mode | Human | Meaning |
|------|-------|---------|
| `interactive` | Present | Live collaboration; approvals via client/channel |
| `grace` | Recently disconnected | Finish in-flight turn; no new client-only tools |
| `autonomous_continuation` | Absent, grace expired | Continue under executor-leaning policy; durable handoffs on block |
| `observational` | Present, read-only | Inspect a run without owning its turns |
| `frozen` | Panic / handoff | Turn cancelled; worktree checkpointed; awaiting disposition |

Why this matters for safety: when a `pair` session loses its client, the session keeps running as a **governance transition**, not a silent flip to `work` — so memory scope and approval semantics never change out from under a run. Governance changes supervision; **it must never be used to launder trust**. Every transition is a durable, model-visible event.

Per turn, Den computes the effective policy as a product — `TrustProfile × Governance × Armature × RunAuthContext` — and enforces the resulting tool roster, memory write target, and approval class. The model never infers its own permissions.

## Approvals and human gating

- **Armature-local actions** (filesystem, git, terminal in `pair`) flow through client obligations: Den emits the obligation, the armature renders permission UI, the human approves, the tool executes, and the same turn resumes. The core coordinator — not the edge — decides when continuation is legal ([ADR-0048](docs/decisions/adr-0048-core-turn-client-obligation-coordinator.md)).
- **Approvals are authoritative in Den**, never "maybe the disconnected client will answer" ([ADR-0026](docs/decisions/adr-0026-work-handoff-and-human-escalation.md)).
- **Background work requires approved Docket tasks**; `work` cannot self-approve, and acceptance criteria define what "done" means ([ADR-0034](docs/decisions/adr-0034-jobs-and-tasks-work-management.md)).
- Humans can always override curation: correct, delete, approve, or reshape what the system believes.

## Execution isolation for `work`

When `work` needs shell, filesystem, or network access, it runs in a Den-managed sandbox ([ADR-0037](docs/decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md)):

- Each run gets a **paired workspace + egress-gateway** container pair on an isolated network; parallel runs are isolated from each other.
- **Credentials are injected at the egress gateway**, not placed in the workspace. The model never directly chooses or sees upstream credentials; git/auth actor selection follows `RunAuthContext` (e.g., the Bear service identity commits, the requesting human opens the PR).
- `chat` **never** gets a sandbox — it delegates to a `work` run. `pair` defaults to the client armature. `curate` and `watch` have narrow in-process tool rosters only.

## Enforcement, not vibes

The boundaries above are enforced structurally, which is what makes them trustworthy:

- Stance capability boundaries are **policy-enforced by Den**, not informal prompt guidance ([docs/architecture/den-bear-spec.md](docs/architecture/den-bear-spec.md)).
- Tool access is **descriptor-owned**: which tools are visible, where they execute, and under what approval class is metadata resolved by Den, never inferred from tool-name strings or prompt heuristics.
- Control data never travels as strings in transcript text; typed protocol fields and events carry it (see `AGENTS.md`, "Typed Boundaries and String Hygiene").
- Tool calls and results are **replayable transcript state** — there is always an auditable record of what ran and what happened.
- Den tells the model which single instruction applies (e.g., whether a work surface is writable) rather than asking the model to choose between permission modes ([docs/architecture/context-compilation-scenarios.md](docs/architecture/context-compilation-scenarios.md)).

## Canonical sources

- [docs/architecture/bear-stances.md](docs/architecture/bear-stances.md) — the five stances, responsibilities, and limits
- [docs/architecture/interactive-stances-and-role-axes.md](docs/architecture/interactive-stances-and-role-axes.md) — stance vs channel vs armature vs governance
- [ADR-0039](docs/decisions/adr-0039-trust-profiles-and-governance.md) — trust profiles and governance
- [ADR-0037](docs/decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md) — sandbox, egress gateway, upstream auth
- [ADR-0026](docs/decisions/adr-0026-work-handoff-and-human-escalation.md) — handoff and human escalation
- [ADR-0034](docs/decisions/adr-0034-jobs-and-tasks-work-management.md) — Docket jobs/tasks and acceptance criteria
- [docs/architecture/memory-model.md](docs/architecture/memory-model.md) — memory boundaries and promotion
- [docs/GLOSSARY.md](docs/GLOSSARY.md) — trust stance, governance, armature, continuation bias
