# Brief: Bear Management UI

**Status:** Design brief (target design, not current implementation)
**Companion:** [bear management model](bear-management-model.md) — the information architecture and the capabilities/resources model behind this brief.
**Related:** [SAFETY.md](../../SAFETY.md) (stance model), [PORTABILITY.md](../../PORTABILITY.md), [bear package](../guides/bear-package.md), [memory model](../architecture/memory-model.md), [bear stances](../architecture/bear-stances.md), [ADR-0040 — Connections](../decisions/adr-0040-connections-and-work-surface-presentation.md), [ADR-0034 — Jobs and tasks](../decisions/adr-0034-jobs-and-tasks-work-management.md)

## Design thesis

This UI is where *"an AI you govern, not one you rent"* stops being a slogan and becomes literally operable. It is not a settings page bolted onto a chatbot — it is the **control surface**, and it is the single most important place the product either keeps or breaks its promises. A management UI that is opaque, read-only, or that hides what the Bear can reach would silently refute the entire pitch, no matter how good the assistant is.

The test to hold every screen against: **does this let a non-expert _see, understand, and change_ what the Bear is, what it knows, what it can reach, and what it has done?** If a screen only lets you _see_, it fails; inspection without the power to change is the hosted-assistant experience we're defining ourselves against.

## Design principles (non-negotiable — each embodies a verb)

1. **Legibility over magic (audit).** Nothing the Bear believes or did is presented without a traceable source. Every memory, belief, and action carries provenance you can reach. No screen ever says, in effect, "the AI decided."
2. **Reach is legible and granular (bound).** What each stance can reach and do — its tools, its connected resources, its data access — is visible and set **per stance**, never global and never buried in raw config. Where a configuration *you* composed carries a risky combination (untrusted input + private-data reach + outbound action), the UI surfaces it as an **honest caution to review** — an observation about your setup, never a guarantee the system makes on your behalf.
3. **Correction is a first-class gesture (correct).** Wherever you can inspect memory, you can fix or forget it in the same view. Reading without editing betrays "you are the editor of its mind."
4. **Ownership is exercisable, not theoretical (own/move).** Export, download, and migration are obvious, honest, and never dark-patterned. What travels and what stays is shown plainly.
5. **Cause and effect is traceable.** From a conversation you can see the memories it formed and the work it dispatched; from a memory you can reach the conversation that formed it. The record is a graph you can walk, not disconnected logs.
6. **No irreversible surprises.** Changes that weaken isolation or delete memory preview their effect in plain language before they happen. Destructive actions are clear without being frightening.
7. **Honest about canonical vs derived.** Canonical memory is truth; recall indexes are rebuildable and labeled as such. The UI never dresses derived state up as authoritative.
8. **Portability is legible where you stand (own).** A user should be able to tell, on whatever screen they're on, what of the Bear is theirs to take and what is bound to this Den — not discover it only at export. This is a requirement on the *information*, not a mandate of mechanism. Explicitly **do not** default to stamping every item with a blunt "portable / attached" badge; the elegant expression is still to be found, and may live in framing, grouping, wording, or how re-attach is handled on import rather than a per-row tag. The test is whether a user is ever *surprised* at export — not whether a flag exists. (Current leading expression: the ownership-grouped nav in the IA section below.)

## Information architecture (top-level areas)

The nav is **one continuous list, softly grouped by ownership** — not two modes you switch between. The grouping itself makes the portability boundary legible (the leading candidate for principle 8's mechanism): what is intrinsically the Bear's and travels with it, versus what is wiring and record that stays on this Den.

**Overview** — health, recent activity, and what's pending your review. (Spans both groups.)

**Yours** — travels with the Bear:
- **Identity & charter** — name, slug, charter, per-stance model choice.
- **Memory** — the mind: the curated store the Bear knows itself by. Kept prominent.
- **Skills** — reviewed, owned procedures that direct how the Bear works. Owned knowledge, not host wiring — which is why they sit here, with the mind, rather than with tools and connections.

**This Den** — stays here for now:
- **Capabilities** — what the Bear can *do*: tools and MCP servers (which supply tools), granted per stance.
- **Resources** — what the Bear *acts on and reads*: external resources reached through **Connections** (repositories, documents, servers) and internal ones (Cabinet, Docket work surfaces).
- **Activity** — what the Bear *did*: **Conversations**, **Jobs**, and **Cabinet activity**.
- **People** — membership and access.

**Portability** — export and import. (The mover; belongs to neither group.)

Two things make this honest rather than clever. The second group is labeled by **ownership** ("This Den — stays here for now"), never by infrastructure ("server"): the split should read *this is intrinsically yours; that's just where it lives today*, which reinforces the pitch instead of centering the plumbing. And the grouping is **soft** — everything stays in one visible nav, so routine tasks never require deciding "which half is this in." The Memory/Activity pairing still anchors it (the mind travels, the record stays), and the deeper model behind Capabilities and Resources — the two axes bridged by connections — is in the [companion doc](bear-management-model.md).

## Primary use cases (the test cases — actor, motivation, success bar)

1. **Stand it up safely** *(setup).* A new user provisions a Bear. *Motivation:* get to a useful assistant fast — but not by blindly handing a stranger-in-a-box access to everything. *Succeeds if:* they reach a working Bear in minutes **and** can see, before it touches anything real, what each stance may reach and do, with safe legible defaults they didn't have to hand-configure.

2. **Check before I trust** *(audit).* The user is about to act on something the Bear asserted. *Motivation:* don't act on a confident hallucination. *Succeeds if:* from any assertion they can reach its provenance — author stance, date, source, lineage — in a click or two, and can tell "grounded in something I gave it" from "inferred."

3. **Fix what it's wrong about** *(correct/forget).* The Bear holds a wrong or unwanted belief and keeps acting on it. *Motivation:* I edit its mind; I don't beg it to please forget. *Succeeds if:* the user finds the offending memory and corrects or forgets it in the same view, with visible confirmation it's gone from what the Bear will use next turn — no forget-theater.

4. **Grant a capability and see the risk I'm composing** *(bound).* The user wants the Bear to gain a capability or reach a resource (browse the web, connect a repo). *Motivation:* capability without it becoming a weapon against me. *Succeeds if:* granting is per stance, shows what the capability or resource exposes (private-data reach / untrusted input / outbound action), and if the grant composes a risky combination the UI says so as a review prompt — an observation about your configuration, not a safety guarantee.

5. **Approve what it wants to remember (or install)** *(curate/own).* The Bear proposes durable memories, or a new skill, from recent activity. *Motivation:* stay in control of what my assistant keeps and gains. *Succeeds if:* a review queue lets the user approve, edit, or reject proposed memories and self-proposed capabilities before they take effect.

6. **Trace cause and effect** *(audit).* From a conversation, the user wants to see what the Bear learned and did as a result. *Motivation:* understand how a belief or an action came to be. *Succeeds if:* a conversation links to the memories it formed (with dates) and the jobs it dispatched, and each memory links back to its source conversation.

7. **Direct the work** *(dispatch).* The user wants to create, dispatch, or reprioritize a job. *Motivation:* put the Bear to work and stay in charge of what it's doing. *Succeeds if:* Jobs is an active surface — create, dispatch, prioritize — not a read-only log, and each job's runs are reviewable and linked back to the conversations that spawned them.

8. **Take it with me / back it up** *(own/move).* The user wants to export, migrate hosts, or keep a backup. *Motivation:* no lock-in; it's mine. *Succeeds if:* export is obvious and honest about what travels vs stays, produces a real file, captures recent memory first, and import clearly walks re-attaching secrets, membership, and model remapping.

9. **Is it healthy and behaving?** *(status / trust check).* A routine check-in. *Motivation:* quiet confidence it's working and hasn't drifted. *Succeeds if:* one screen shows health, what it's learned and done recently, and what's pending the user's review — without digging.

## Screens & must-not-be-left-out details

**Overview.** Health (model gateway reachable; recall index state, labeled rebuildable); recent activity across stances; a **pending-review count** as a call to action; identity and active stances at a glance.

**Memory (kept prominent).** Browse by logical scope (`core/`, per-stance, shared). Each record shows content, scope, **author stance, dates (created / valid-from / invalid-at), provenance lineage** (proposal → promotion), and a link to the **source conversation**. Per-record actions: **correct** (honest supersession, not silent overwrite), **forget/invalidate** (confirm + visible effect), **promote** (curate). A **review queue** for proposed memories and watch observations awaiting approval. Search (hybrid when recall is on; keyword fallback honestly labeled). Entity view: people/projects/things the Bear knows, their relations and resolution/trust state, with curate-only merge/split/correct. Canonical memory and derived recall are visually distinct.

**Identity & charter.** Name, slug (human handle), charter/purpose text. Per-stance **model selection**, framed as *the engine is swappable and changing it does not change the mind* — never as "upgrading your Bear." Charter edits recompile prompts; say so honestly (takes effect next turn).

**Skills** *(Yours).* The reviewed, owned procedures that direct how the Bear works — grouped with Memory, not with tools, because they are owned knowledge that travels. Per skill: what it directs, its source and trust, and role applicability (which stances use it). **Self-proposed skills** (bear-authored) flow through the review queue — no stance self-installs executable capability. Approved skills travel with the Bear as package artifacts.

**Capabilities** *(This Den).* Per stance, the granted tools and MCP servers (which supply tools). Each item shows *what it exposes* (private-data reach / untrusted input / outbound), and for credentialed tools, *which connection enables it*. Add from the operator catalog; remove is first-class. Risky combinations are flagged as **review**, never as a safety guarantee. Secrets are named, never shown. A grant's *description* travels with the Bear; the connection, secret, and running server behind it stay on this Den and are re-attached on import — the screen should make that split feel natural, not a surprise sprung at export.

**Resources.** External resources as typed cards (repository, document, server) reached through **Connections**; internal resources (Cabinet, Docket work surfaces). Granted per stance. Connections are set up once (owner-scoped, reused across Bears) and surfaced as the *enabler* behind credentialed tools. Secrets named, never shown, never exported. The Bear's *knowledge* of a resource (anchors and overviews in `core/`) travels as memory; the resource itself and the connection to it stay — a distinction worth surfacing here, not only at export.

**Activity.** Three streams:
- **Conversations** — chat/pair dialogue. Each conversation is a **hub**: it links to the memories it formed (and when) and the jobs it dispatched. This is where a user understands what and when the Bear learned.
- **Jobs** — structured work (Docket). Create, dispatch, and prioritize; review runs. Each run links back to its source conversation and forward to its outputs.
- **Cabinet activity** — the Bear's creates, edits, and comments in the shared knowledge base. Link out to Cabinet for the full and ownership view.

**People** (multi-user). Who may use this Bear; add/remove; roles. Make clear membership does **not** travel with an export.

**Portability.** Export with tier choice (cognition / operator snapshot), the explicit **what-moves / what-stays table**, a **pre-export curation-flush** step (so recent memory is captured before the snapshot), and a real download. Import validates versions, remaps models to the target host, re-provisions host bindings, and presents a re-attach checklist for secrets and membership. Honest throughout that transcripts, secrets, and host bindings stay behind.

**Setup / onboarding.** Shortest honest path to a working Bear: name, charter, pick a model — done, with capabilities and reach pre-set sanely **and shown**, so the first impression is "I can see what this thing can do." Progressive disclosure: legible surface, depth on demand.

## What would betray the promise (red-team against these)

- A **read-only** memory viewer — inspection without correction is the rental experience.
- Capability grants or resource reach reachable only via raw JSON/config — betrays *bound*.
- Risk composition presented as a **guarantee** ("safe") rather than an honest, reviewable observation about the user's own configuration — the overclaim to avoid.
- Memory shown with **no provenance** — betrays *audit*.
- A "forget" that only hides in the UI while the Bear still uses the memory — betrays *correct*.
- Export that's hidden, lossy, or dishonest about contents; **any path where secrets or transcripts leak into a package** — betrays *own* and *bound* at once.
- Derived recall presented as canonical truth.
- **Auto-remembering, or auto-installing capability, with no review** — betrays *own/control*.
- Framing a model change as "upgrading your Bear" — conflates the swappable engine with the owned mind, which is precisely our differentiator inverted.
- **Activity as a flat, disconnected log** — runs detached from their job, conversations detached from the memories they formed. Things feel buried when they're severed from their organizing parent, not merely one level too deep.
- **Portability as a reveal, not ambient** — a user first learning at the export screen that a connection, secret, or Cabinet reference won't travel. What's yours to take should be legible where you stand. (And the remedy is not a blunt per-item badge — see principle 8.)

## Honesty notes (design now, ship as backend lands)

- **Capabilities and Resources** — skills, MCP servers, connections, and Cabinet are not yet built. This reflects the intended model, aligned with [ADR-0037](../decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md) and [ADR-0040](../decisions/adr-0040-connections-and-work-surface-presentation.md) connections, the [skills plan](../roadmap/SKILLS_IMPLEMENTATION_PLAN.md) catalog/attach/review pattern, and the MCP catalog pattern in [PLAN.md](../roadmap/PLAN.md), and extended with the capabilities-vs-resources framing in the [companion model](bear-management-model.md).
- **Activity / Jobs** — Docket jobs/tasks/runs exist ([ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md), [DOCKET_IMPLEMENTATION_PLAN.md](../roadmap/DOCKET_IMPLEMENTATION_PLAN.md)), but the `work` stance sandbox is Phase 7 (unbuilt). Chat/pair **conversations are live** and are the history to un-bury first.
- **Cabinet** folds Garage: Garage stays the storage backend, artifacts are presented to users as Cabinet items. Cabinet itself is not yet built.
- **Portability** export/import depends on entity-layer Phase 7; the pre-export curation flush is spec'd but unbuilt ([BEAR_ENTITY_LAYER_IMPLEMENTATION_PLAN.md](../roadmap/BEAR_ENTITY_LAYER_IMPLEMENTATION_PLAN.md) Phase 7).
- The **clean forget/correct gesture** relies on supersession primitives that exist plus the planned memory-surfaces UX ([MEMORY_SURFACES_IMPROVEMENT_PLAN.md](../roadmap/MEMORY_SURFACES_IMPROVEMENT_PLAN.md)); the target is a single legible action, not a hand-rolled edit.
