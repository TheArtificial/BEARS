# Bear management: information architecture and capability/resource model

**Status:** Design doc (target model, not current implementation)
**Companion:** [Bear management UI brief](bear-management-ui-brief.md) — principles, use cases, and per-screen detail.
**Related:** [ADR-0040 — Connections and work-surface presentation](../decisions/adr-0040-connections-and-work-surface-presentation.md), [ADR-0037 — Work sandbox, egress, upstream auth](../decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md), [ADR-0034 — Jobs and tasks](../decisions/adr-0034-jobs-and-tasks-work-management.md), [skills plan](../roadmap/SKILLS_IMPLEMENTATION_PLAN.md), [bear package](../guides/bear-package.md), [bear stances](../architecture/bear-stances.md)

This doc records the conceptual model the [UI brief](bear-management-ui-brief.md) is built on. Several pieces (skills, MCP servers, connections, Cabinet, the `work` sandbox) are **not yet built**; this is the intended structure they should land into, aligned with the ADRs above and extended where those are incomplete.

## Information architecture at a glance

| Group | Area | What it answers | Travels? |
|-------|------|-----------------|----------|
| — | **Overview** | Is it healthy, what has it done lately, what needs me? | Spans (host dashboard; identity portable) |
| Yours | **Identity & charter** | Who is it? | **Travels** |
| Yours | **Memory** | What does it know — and is that right? | **Travels** — the mind |
| Yours | **Skills** | How does it work (owned procedures)? | **Travels** |
| This Den | **Tools** | What can it *do* (built-in, local, remote/MCP)? | Grant travels; server and secret stay |
| This Den | **Connections** | What accounts does it reach through? | Stays — secrets never travel; re-attached on import |
| This Den | **Resources** | What can it *act on and read*? | Knowledge travels; the resource and its connection stay |
| This Den | **Activity** | What *did* it do? | **Stays** — the record |
| This Den | **People** | Who can use it? | Stays |
| — | **Portability** | How do I back it up or move it? | — (the mover) |

The nav is **one continuous list, softly grouped by ownership** — *Yours* (travels with the Bear) and *This Den* (stays here for now) — not two modes. The grouping is the leading candidate for making portability legible (brief principle 8) without a per-item badge. Label the second group by ownership honesty, never "server": the split should read *this is intrinsically yours; that's just where it lives today*.

The "travels" column is the **conceptual boundary**, not a UI spec; the test is only whether a user is ever surprised at export. Note that Tools and Resources each split *internally* — the grant or the knowledge travels while the live wiring (connection, secret, running server, the resource itself) stays. Those items sit under *This Den* while their portable counterpart (a skill, a memory) sits under *Yours*, so the soft grouping still has to make that seam feel natural — it is the case most likely to surprise.

Two of these are the load-bearing pillars, and they are deliberately paired:

- **Memory** — what the Bear *learned*. Curated, canonical, **portable** — the mind. It travels in the bear package.
- **Activity** — what the Bear *did*. Conversations, jobs, and Cabinet activity. **Host-side**, forensic, does **not** travel.

Keeping these distinct is what lets us say honestly that *a Bear is its memory, not its transcript*: the transcript gets a first-class home as the doing-record without being mistaken for the mind.

In the IA these pillars head a soft ownership grouping. **Memory** and **Skills** — owned, portable knowledge — sit with identity under *Yours*; **Activity**, along with tools, connections, and resources, sits under *This Den*. The grouping is one nav, not two modes, and is the leading way to make the portability boundary legible without per-item flags.

## Two axes of reach: capabilities × resources

What a Bear can reach is two orthogonal things, not one. Conflating them (an earlier draft folded everything into "capabilities") hides half the risk surface.

### What the Bear can *do* (the capability axis)

Three kinds of capability-expander, distinguished by *how* they extend the base layer of built-in and armature-local tools:

| Kind | Verb | What it adds |
|------|------|--------------|
| **Connection** | **enables** | Credentialed reach that lets existing tools act on *your* stuff (connect GitHub → git tools work on your repos). |
| **MCP server** | **supplies** | A bundle of new callable tools. |
| **Skill** | **directs** | A reviewed procedure/knowledge package shaping *how* tools are used. |

Note the seams: a tool-bearing skill can also *supply* capability (so it is review-gated), and a connection is the *visible half* of a bridge — see below.

These three also sort cleanly by ownership: **direct** (skills) is owned, reviewed knowledge that travels with the Bear — so in the IA it sits with memory under *Yours* — while **enable** (connections) and **supply** (MCP servers) are host infrastructure that stays, grouped with resources under *This Den*. That sort is why "Capabilities" as a single lumped area dissolved: it had been mixing a mind-thing (skills) with plumbing (tools, connections).

So in the UI this axis is not one area. **direct** (skills) is its own area under *Yours*. **supply** and the base tools become **Tools** under *This Den* — one list of every tool the Bear can call, each tagged by **origin** (built-in, armature-local, or remote via an MCP server), because origin is the trust signal, not decoration. **enable** (connections) is surfaced through Resources. One caveat keeps the axes clean: an MCP server can also expose non-tool primitives, and if Bear Den ever consumes MCP *resources* (not just tools), those belong on the Resources axis, not Tools.

### Resources — what the Bear *acts on and reads*

The durable things a Bear works against (work surfaces, in model-facing terms — kept internal per [ADR-0040](../decisions/adr-0040-connections-and-work-surface-presentation.md)):

- **External**, reached through a **Connection** — repositories, documents, servers, designs. Shown as typed cards.
- **External**, governed by **policy** rather than a connection — e.g. the open web, under a web-source policy.
- **Internal** — Cabinet, Docket work surfaces, and the Bear's own memory.

### Connections are the bridge

A connection is not simply a "tool-enabler" — that is the visible half. Underneath, a connection is an **authenticated bridge to external resources**: it exposes specific resources (which repos, which docs) *and* enables the credentialed tools that act on them. It is owner-scoped and set up once, then reused across Bears and granted per stance. "Connect GitHub, get git tools on your repos" is the right thing to *show* users; the resource axis is what makes the model *correct*.

In the IA, **Connections is its own top-level area** under *This Den* — the accounts you authenticate, distinct from both the Tools they enable and the Resources they reach, and also surfaced *at* those places as the enabler. This refines [ADR-0040](../decisions/adr-0040-connections-and-work-surface-presentation.md), which presented typed external resources under a single "Connections"-labeled area: separating the accounts (Connections) from the things reached (Resources) is clearer now that Resources is its own area and Tools reference connections too. Connections stay on this Den — secrets never travel — and are re-attached on import.

### Why two axes matter: honest risk, not a promise

The lethal trifecta — untrusted input + private-data access + outbound action — composes across **both** axes. The "private data" leg *is* the resource axis (your repo, your docs, Cabinet). A UI that assessed only capabilities, or only memory, would be blind to half of it. So risk is surfaced at the point of granting, across capabilities and resources together, as an **honest caution to review a combination you composed** — never as a guarantee the system prevents it, and never claiming to catch every risky case.

## Cabinet (folding Garage)

Cabinet is a wiki-style, long-lived knowledge base and artifact store that humans and Bears use together. **Garage** is folded in: it remains the object-storage backend, but users see artifacts as items *in Cabinet*, not a separate concept.

Cabinet appears in the Bear UI in the two places the two-axis model predicts, plus a link out:

- In **Resources** — Cabinet is an internal resource a stance can be granted read/write on.
- In **Activity** — the Bear's creates, edits, and comments show as a Cabinet-activity stream (the internal-resource case of the general "activity on a resource" pattern that will later cover repos, docs, etc.).
- A **link out** to Cabinet itself for the full contents and ownership view; the Bear UI does not reimplement Cabinet.

## The shared lifecycle

Tools, skills, resources, and connections all follow one lifecycle:

**catalog → attach (per Bear, per stance) → materialize → govern**

- **Catalog** — an operator/org library of *trusted* MCP servers and skills, and the set of connectable providers, each carrying source and trust flags.
- **Attach** — the per-Bear, per-stance act of granting one. Grants are never global: a connection granted to `pair` is not available to `watch`.
- **Materialize** — Den reconciles the attachment into the runtime and surfaces drift.
- **Govern** — trust of source, secret status, review of self-proposed capability, revocation.

The boundary that shapes every screen: **Den stores metadata and policy — not processes, not secrets.** The host platform runs MCP server processes; the host secret store holds credentials (surfaced *named, never shown*); `bear-armature` executes local tools for `pair`. The UI reflects *state* (connected / attached / running / missing secret / drifted), it does not orchestrate. And **anything the Bear proposes for itself** — a bear-authored skill, a new capability — passes through the review queue before it takes effect: the same ownership posture as approving a proposed memory.

## Connective tissue: conversation ↔ memory ↔ jobs

The record is a graph you can walk, not disconnected logs. A **conversation** is the hub:

- forward to the **memories it formed** (with dates), and
- forward to the **jobs it dispatched**.

And a **memory** links *back* to its **source conversation** (the provenance already on the memory record). This closed loop is what makes ownership and audit feel real: from any belief you reach the moment it formed, and from any conversation you see what the Bear learned and did as a result.

## Build posture

Build simple first: plain semantic HTML styled from the existing CSS, no fancy chrome, and **no JavaScript for basic functionality** — every core action works server-rendered, with JS as progressive enhancement only. **Responsive disclosure** (breadth by viewport, distinct from depth-on-demand progressive disclosure) lets a screen's core stand alone on small screens while larger screens reveal more via CSS; the Overview is the main beneficiary (see its frame — a compact core, plus model usage/budget, memory statistics, and activity-over-time disclosed when there's room). Full statement: [UI brief, Implementation posture](bear-management-ui-brief.md#implementation-posture).

## Screen map (text frames)

Low-fidelity frames: each box lists the elements on a screen as a flat set — no layout or visual hierarchy is implied. The nav groups these softly as *Yours* / *This Den* (see the IA); the map implies neither that grouping nor any layout. Edges below are cross-links, not navigation order.

```
┌─ Overview (home) ──────────────┐   ┌─ Memory ───────────────────────┐
│ core (all sizes):              │   │ · browse by scope              │
│ · health                       │   │ · search (hybrid / keyword)    │
│ · pending reviews              │   │ · entity view                  │
│ · identity + active stances    │   │ · review queue                 │
│ · recent activity              │   │ · [open a record]              │
│ disclosed (larger screens):    │   └────────────────────────────────┘
│ · model usage & budget         │
│ · memory statistics            │
│ · activity over time           │
└────────────────────────────────┘
┌─ Memory record ────────────────┐   ┌─ Tools ────────────────────────┐
│ · content                      │   │ · one list, tagged by origin   │
│ · scope + author stance        │   │ · built-in / local / remote    │
│ · dates                        │   │ · what each exposes            │
│ · provenance lineage           │   │ · enabled-by connection        │
│ · source conversation          │   │ · add from catalog / remove    │
│ · correct / forget / promote   │   │ · per stance                   │
└────────────────────────────────┘   └────────────────────────────────┘
┌─ Resources ────────────────────┐   ┌─ Connections ──────────────────┐
│ · external (typed cards)       │   │ · authenticated accounts       │
│ · internal (Cabinet, Docket)   │   │ · owner-scoped, reused         │
│ · granted per stance           │   │ · secret status (named only)   │
│ · web under policy             │   │ · connect / revoke             │
└────────────────────────────────┘   └────────────────────────────────┘
┌─ Activity ─────────────────────┐   ┌─ Conversation ─────────────────┐
│ · Conversations                │   │ · transcript                   │
│ · Jobs                         │   │ · memories formed (+ dates)    │
│ · Cabinet activity             │   │ · jobs dispatched              │
│ · [open a conversation / job]  │   └────────────────────────────────┘
└────────────────────────────────┘
┌─ Jobs ─────────────────────────┐   ┌─ Cabinet activity ─────────────┐
│ · create / dispatch            │   │ · creates / edits / comments   │
│ · prioritize                   │   │ · link out to Cabinet          │
│ · runs (linked to convo)       │   └────────────────────────────────┘
└────────────────────────────────┘
┌─ People ───────────────────────┐   ┌─ Portability ──────────────────┐
│ · membership                   │   │ · export tier                  │
│ · roles                        │   │ · what moves / what stays      │
│ · does not travel on export    │   │ · pre-export curation flush    │
└────────────────────────────────┘   │ · import re-attach checklist   │
                                      └────────────────────────────────┘
┌─ Identity & charter ───────────┐   ┌─ Skills ───────────────────────┐
│ · name / slug / charter        │   │ · owned procedures             │
│ · per-stance model (swappable) │   │ · directs how tools are used   │
└────────────────────────────────┘   │ · source / trust / roles       │
                                      │ · review self-proposed         │
                                      └────────────────────────────────┘
```

Cross-links (the graph, not the nav):

- Memory record ⇄ Conversation — a memory names its source conversation; a conversation lists the memories it formed.
- Conversation → Jobs — a conversation lists the jobs it dispatched; a job run names the conversation that spawned it.
- Tools → Connections — a credentialed tool shows the connection that enables it; a missing connection prompts to connect.
- Resources → Connections — an external resource is reached through the connection that authenticates it.
- Resources ⇄ Activity — Cabinet (and later repos) are granted in Resources and generate a Cabinet-activity stream in Activity.
- Cabinet activity → Cabinet (external) — link out for the full/ownership view.
- Overview → review queue (Memory) / Jobs / Activity — the pending-review count and recent activity are entry points.

## Build status

| Piece | Status |
|-------|--------|
| Memory (canonical, provenance, correct/forget) | Primitives exist; memory-surfaces UX planned |
| Conversations (chat/pair) | Live — the activity to un-bury first |
| Jobs (Docket) | Jobs/tasks/runs exist; `work` sandbox is Phase 7 (unbuilt) |
| Tools (built-in, armature, MCP-as-remote) and Skills | Not built; catalog/attach/review pattern per skills plan and PLAN.md |
| Resources / Connections | Not built; per ADR-0037 / ADR-0040 |
| Cabinet (folds Garage) | Not built |
| Portability | Not built; entity-layer Phase 7 + pre-export curation flush |
