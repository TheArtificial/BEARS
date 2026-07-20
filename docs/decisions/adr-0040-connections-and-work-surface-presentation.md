# ADR-0040 — Connections and user-facing work-surface presentation

**Status:** Accepted (2026-06-13)
**Deciders:** Hans
**Related:**
- [ADR-0006 — Bear work surfaces](adr-0006-bear-work-surfaces.md)
- [ADR-0024 — Terminology: actuators, resources, role names](adr-0024-terminology-actuators-resources-and-role-names.md) (this ADR resolves its §2)
- [ADR-0037 — Work sandbox, egress gateway, and upstream auth](adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md)
- [ADR-0028 — Environment affordance and resource boundaries](adr-0028-environment-affordance-and-resource-boundaries.md)
- [ADR-0039 — Trust profiles and governance](adr-0039-trust-profiles-and-governance.md)
- [`work-surfaces-and-conversations.md`](../guides/work-surfaces-and-conversations.md)

## Context

[ADR-0006](adr-0006-bear-work-surfaces.md) chose **work surface** as the primary term for the durable thing a Bear acts on (a repo, service, deployment, Mission, Docket project, research area). The term works well as an *architecture* and *model-facing* concept: it is domain-agnostic and captures that one durable surface can have multiple manifestations.

It is a poor *user-facing* term: "surface" is an abstract metaphor a user has to be taught. The pressure becomes concrete with upcoming UI where a person will, at the **Den level**, authenticate and attach external resources for a Bear to use across trust profiles and governance — for example: clone a GitHub repository, link a Figma project, configure a server over SSH, or connect Google Docs. "Repo" is far too narrow for that heterogeneous set.

Two further facts shape the decision:

- [ADR-0037](adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md) already introduced owner-scoped **Connections** for upstream auth and credential injection at egress. The provider-auth concept exists.
- [ADR-0024](adr-0024-terminology-actuators-resources-and-role-names.md) (Proposed, 2026-05-23) proposed renaming work surface → "resource" generally. That proposal was **not adopted** — later ADRs ([ADR-0036](adr-0036-bear-profile-registry.md), [ADR-0039](adr-0039-trust-profiles-and-governance.md)) keep "work surface" and keep the `pair`/`curate` role names ADR-0024 also tried to change. This ADR resolves the work-surface portion explicitly.

## Decision

### 1. "Work surface" stays internal; it is not a user-facing label

`work surface` remains the architecture and **model-facing** term, unchanged in code, schema, and tool/runtime prose: `work_surface_ref`, `session_info.work_surface`, the `core/tools/work_surface/` module, and ADR-0006 anchors are all retained. Models continue to reason about work surfaces. The term is simply **never shown to human users**.

### 2. Two concepts, named separately (the split that "work surface" was hiding)

- **Connection** — a Den-level, **owner-scoped authenticated link to an external provider**: a GitHub/GitLab account, a Figma account, a Google account, an SSH host + credential. Set up once; reusable across many resources, Bears, trust profiles, and governance. This formalizes ADR-0037's Connections as a first-class entity.
- **Resource (a work surface)** — the specific thing a Bear acts on, reached **through** a Connection when it is externally backed.

Cardinality: one Connection → many resources for GitHub/Figma/Google; roughly 1:1 for SSH. **Not every work surface has a Connection** — internally-backed surfaces (a Cabinet Mission, a Docket project) have no external provider link and are surfaced through their own areas.

### 3. User-facing presentation: typed cards, no single umbrella noun

Externally-connected resources are presented under a **Connections** area as concrete, **typed** items rather than one abstract umbrella word:

- **Repository** (GitHub/GitLab/Gitea/…)
- **Design** (Figma)
- **Server** (SSH)
- **Document** (Google Docs/Drive)
- …extensible per provider.

Do **not** expose `work surface`, `resource` (as a label), or `Space` to users. The umbrella stays internal; the UI shows what the thing actually is.

### 4. Translation lives at the presentation layer

The model reasons in work surfaces; the UI maps a work surface's anchor kind ([ADR-0006](adr-0006-bear-work-surfaces.md)) to a typed card and its Connection. Labels are descriptor-owned (per `AGENTS.md`: no scattered alias `match` arms or hardcoded allowlists), so adding a provider adds a descriptor, not new conditionals.

## Consequences

- Users get familiar, concrete labels (Repository, Design, Server, Document) and a well-understood **Connections** section, instead of an abstract umbrella.
- **Connection** becomes a first-class Den-level entity (auth/credential scope) distinct from the resources it exposes, consistent with ADR-0037.
- [ADR-0024](adr-0024-terminology-actuators-resources-and-role-names.md) §2 ("work surface → resource") is **superseded**: "work surface" remains the architecture term, and "resource" is only a descriptive common noun, not a UI or canonical label.
- No code/schema churn: this is a presentation and entity-modeling decision, not a rename of `work_surface_*`.

## Naming

| Concept | Internal / code / model-facing | Human UI |
|---------|--------------------------------|----------|
| Durable acted-upon resource | **work surface** (`work_surface_ref`, `session_info.work_surface`, anchors) | **typed card**: Repository / Design / Server / Document / … |
| External provider auth link | **connection** (owner-scoped) | **Connections** |
| Active working instance | workspace session / run ([ADR-0039](adr-0039-trust-profiles-and-governance.md)) | session / run |

Do not introduce a single user-facing umbrella noun for work surfaces; prefer typed labels grouped under Connections.

## Follow-ups (not decided here)

- The **Connection** entity schema: provider, owner scope, and credential handling under the ADR-0037 egress/credential-injection design.
- A typed work-surface-kind registry mapping ADR-0006 anchor kinds → UI card types and their Connection requirement.
- Whether internally-backed surfaces (Cabinet Mission, Docket project) appear in the same UI area or remain in Cabinet/Docket surfaces.
