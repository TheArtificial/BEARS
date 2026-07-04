# FAQ

Short answers to common architecture questions. For consolidated overviews see [ARCHITECTURE.md](ARCHITECTURE.md), [MODEL_EXPERIENCE.md](MODEL_EXPERIENCE.md), [SAFETY.md](SAFETY.md), and [PORTABILITY.md](PORTABILITY.md); for detail see [docs/architecture/den-runtime.md](docs/architecture/den-runtime.md) and [docs/roadmap/PLAN.md](docs/roadmap/PLAN.md).

## Why does web chat go through Den?

The browser is untrusted, so **Den is the gate**: it authenticates the user, checks bear membership, resolves the Bear stance/session context, executes the native agent loop, and enforces Den-hosted tool policy. Channels bring their own app identity and signing, but they should reuse Den run services rather than bypass Den authorization.

## Is a Bear five different agents?

No. A Bear is one assistant identity. The five stances (`chat`, `pair`, `curate`, `work`, `watch`) are internal trust and capability boundaries over one Den-owned runtime loop — the user always talks to the Bear. See [SAFETY.md](SAFETY.md) and [docs/architecture/bear-stances.md](docs/architecture/bear-stances.md).

## Can I move a Bear to another Den server?

Yes — that's a design goal. A Bear's cognition (per-Bear `memory.sqlite`) plus its configuration (`manifest.yaml`, optional artifacts) form a portable package; conversations, tasks, membership, and secrets stay on the host. See [PORTABILITY.md](PORTABILITY.md).

## What happens to a pair session when the user disconnects?

The run keeps going as a **governance-mode transition** (`interactive` → `grace` → `autonomous_continuation`), not a switch to a different stance — so memory scope and approval semantics never silently change. See [ADR-0039](docs/decisions/adr-0039-trust-profiles-and-governance-modes.md).
