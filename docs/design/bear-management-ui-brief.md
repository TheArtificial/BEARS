# Brief: Bear Management UI

**Status:** Design brief (target design, not current implementation)
**Related:** [SAFETY.md](../../SAFETY.md) (stance/boundary model), [PORTABILITY.md](../../PORTABILITY.md), [bear package](../guides/bear-package.md), [memory model](../architecture/memory-model.md), [bear stances](../architecture/bear-stances.md)

## Design thesis

This UI is where *"an AI you govern, not one you rent"* stops being a slogan and becomes literally operable. It is not a settings page bolted onto a chatbot — it is the **control surface**, and it is the single most important place the product either keeps or breaks its promises. A management UI that is opaque, read-only, or that hides boundaries would silently refute the entire pitch, no matter how good the assistant is.

The test to hold every screen against: **does this let a non-expert _see, understand, and change_ what the Bear is, what it knows, and what it may do?** If a screen only lets you _see_, it fails; inspection without the power to change is the hosted-assistant experience we're defining ourselves against.

## Design principles (non-negotiable — each embodies a verb)

1. **Legibility over magic (audit).** Nothing the Bear believes is presented without a traceable source. Every memory, belief, and action carries provenance you can reach. No screen ever says, in effect, "the AI decided."
2. **Boundaries are the spine, not an advanced setting (bound).** The stance model is visible on the surface of the UI, not buried in config. A user should see at a glance what each stance can reach and do — and the design must make it *hard* to accidentally create a stance that combines untrusted input, private memory, and outbound action.
3. **Correction is a first-class gesture (correct).** Wherever you can inspect memory, you can fix or forget it in the same view. Reading without editing betrays "you are the editor of its mind."
4. **Ownership is exercisable, not theoretical (own/move).** Export, download, and migration are obvious, honest, and never dark-patterned. What travels and what stays is shown plainly.
5. **No irreversible surprises.** Changes that weaken isolation or delete memory preview their effect in plain language before they happen. Destructive actions are clear without being frightening.
6. **Honest about canonical vs derived.** Canonical memory is truth; recall indexes are rebuildable and labeled as such. The UI never dresses derived state up as authoritative.

## Primary use cases (the test cases — actor, motivation, success bar)

1. **Stand it up safely** *(setup / bound-first).* A new user provisions a Bear. *Motivation:* get to a useful assistant fast — but not by blindly handing a stranger-in-a-box access to everything. *Succeeds if:* they reach a working Bear in minutes **and** can see, before it touches anything real, what each stance may reach and do, with safe legible defaults they didn't have to hand-configure.

2. **Check before I trust** *(audit).* The user is about to act on something the Bear asserted — a past decision, a figure, a claim. *Motivation:* don't act on a confident hallucination. *Succeeds if:* from any assertion they can reach its provenance — author stance, date, source, lineage — in a click or two, and can tell "grounded in something I gave it" from "inferred."

3. **Fix what it's wrong about** *(correct/forget).* The Bear holds a wrong or unwanted belief and keeps acting on it. *Motivation:* I edit its mind; I don't beg it to please forget. *Succeeds if:* the user finds the offending memory and corrects or forgets it in the same view, with visible confirmation it's gone from what the Bear will use next turn — no forget-theater.

4. **Understand and set the boundaries** *(bound).* The user wants the Bear to gain a capability (read email, browse the web) and needs to grasp the risk. *Motivation:* capability without it becoming a weapon against me. *Succeeds if:* there's a legible boundary map, and any change that weakens isolation is explained in plain risk terms at the point of change — not discoverable only by reading docs.

5. **Approve what it wants to remember** *(curate/own).* The Bear proposes durable memories from recent activity. *Motivation:* stay in control of the model it's building of me. *Succeeds if:* a review queue lets the user approve, edit, or reject proposed memories before they become durable.

6. **Take it with me / back it up** *(own/move).* The user wants to export, migrate hosts, or keep a backup. *Motivation:* no lock-in; it's mine. *Succeeds if:* export is obvious and honest about what travels vs stays, produces a real file, captures recent memory first, and import clearly walks re-attaching secrets, membership, and model remapping.

7. **Is it healthy and behaving?** *(status / trust check).* A routine check-in. *Motivation:* quiet confidence it's working and hasn't drifted. *Succeeds if:* one screen shows health, what it's learned and done recently, and what's pending the user's review — without digging.

8. **Give it a tool or source** *(connect capability).* Attach an MCP server, skill, or web source. *Motivation:* extend it without opening a hole. *Succeeds if:* attaching shows the trust implication and which stance receives the capability; secrets are named, never displayed.

## Screens & must-not-be-left-out details

**Overview / status.** Health (model gateway reachable; recall index state, labeled rebuildable); recent activity across stances; a **pending-review count** as a call to action; identity and active stances at a glance.

**Identity & charter.** Name, slug (human handle), charter/purpose text. Per-stance **model selection**, framed as *the engine is swappable and changing it does not change the mind* — never as "upgrading your Bear." Charter edits recompile prompts; say so honestly (takes effect next turn).

**Stances & boundaries (the spine).** A **boundary matrix**: rows are stances (chat / pair / curate / work / watch), columns are *ingests untrusted input? / memory scopes readable / memory scopes writable / capabilities / can act outbound?*. Per-stance detail explaining what it's for and what it deliberately can't reach. Editing a scope or tool previews the effect and **warns explicitly on any isolation-weakening change** in injection-risk terms. A standing invariant check: flag loudly if a configuration would let one stance combine untrusted input + outbound action + unrestricted memory.

**Memory.** Browse by logical scope (`core/`, per-stance, shared). Each record shows content, scope, **author stance, dates (created / valid-from / invalid-at), provenance lineage** (proposal → promotion), and source links. Per-record actions: **correct** (honest supersession, not silent overwrite), **forget/invalidate** (confirm + visible effect), **promote** (curate). A **review queue** for proposed memories and watch observations awaiting approval. Search (hybrid when recall is on; keyword fallback honestly labeled). Entity view: people/projects/things the Bear knows, their relations and resolution/trust state, with curate-only merge/split/correct. Canonical memory and derived recall are visually distinct.

**Capabilities & connections.** Tools, skills, and MCP servers with which stance holds each; attach/detach with trust implication shown. Web-source policy; watch subscriptions. **Secrets are named only, never shown**; status is attached/missing, with a re-attach flow.

**Membership & access** (multi-user). Who may use this Bear; add/remove; roles. Make clear membership does **not** travel with an export.

**Portability.** Export with tier choice (cognition / operator snapshot), the explicit **what-moves / what-stays table**, a **pre-export curation-flush** step (so recent memory is captured before the snapshot), and a real download. Import validates versions, remaps models to the target host, re-provisions host bindings, and presents a re-attach checklist for secrets and membership. Honest throughout that transcripts, secrets, and host bindings stay behind.

**Setup / onboarding.** Shortest honest path to a working Bear: name, charter, pick a model — done, with boundaries pre-set sanely **and shown**, so the first impression is "I can see what this thing can do." Progressive disclosure: legible surface, depth on demand.

## What would betray the promise (red-team against these)

- A **read-only** memory viewer — inspection without correction is the rental experience.
- Boundaries reachable only via raw JSON/config — betrays *bound*.
- Memory shown with **no provenance** — betrays *audit*.
- A "forget" that only hides in the UI while the Bear still uses the memory — betrays *correct*.
- Export that's hidden, lossy, or dishonest about contents; **any path where secrets or transcripts leak into a package** — betrays *own* and *bound* at once.
- Derived recall presented as canonical truth.
- **Auto-remembering everything with no review** — betrays *own/control*.
- Framing a model change as "upgrading your Bear" — conflates the swappable engine with the owned mind, which is precisely our differentiator inverted.

## Honesty notes (design now, ship as backend lands)

- **Portability export/import** UI depends on entity-layer Phase 7 (not yet built); the pre-export curation flush is spec'd but unbuilt ([BEAR_ENTITY_LAYER_IMPLEMENTATION_PLAN.md](../roadmap/BEAR_ENTITY_LAYER_IMPLEMENTATION_PLAN.md) Phase 7). Design it now; ship when the backend does.
- The **work stance** row exists in the boundary matrix, but its capabilities are limited until the sandbox lands ([DEN_RUNTIME_PLAN.md](../roadmap/DEN_RUNTIME_PLAN.md) Phase 7).
- The **clean forget/correct gesture** relies on supersession primitives that exist plus the planned memory-surfaces UX ([MEMORY_SURFACES_IMPROVEMENT_PLAN.md](../roadmap/MEMORY_SURFACES_IMPROVEMENT_PLAN.md)); the target is a single legible action, not a hand-rolled edit.
