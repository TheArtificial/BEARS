# Bear Den — Basic Environment for Agent Runtimes Stack

**Bear Den** hosts durable AI assistants called **Bears**. A Bear is one coherent assistant — with its own memory, charter, tools, and working relationships — that a user can meet in web chat, pair with inside an editor, and trust with reviewed background work. **Den** (Rust, `services/den/`) is the runtime and control plane that makes that possible: it runs the in-process agent loop, owns users↔Bears membership, first-party web chat, BearWire/ACP armature sessions, per-Bear SQLite memory, and Cabinet integration when Outline is deployed.

This repo is a **light monorepo**: `docs/`, `services/den/` for Den, `services/bifrost/` for the model gateway config, `services/garage/` for object storage, app/client packaging, and supporting service assets under `services/`.

## The system in five sentences

1. Den runs **one in-process agent loop** for every kind of work and talks directly to **Bifrost** for inference; ACP/BearWire armatures, web chat, and APIs are edge adapters over the same core.
2. A Bear operates through five **stances** — `chat`, `pair`, `curate`, `work`, `watch` — capability and trust profiles over that single loop, so no single stance combines untrusted input, outbound action, and unrestricted shared memory.
3. **Bear cognition lives in per-Bear SQLite**; conversations, approvals, tasks, identity, and compiled prompts live in **Den Postgres** — a boundary that makes each Bear portable between Den servers.
4. Every turn, Den compiles a bounded **turn context** (compiled stance prompt, projected memory, recall, runtime supplements, transcript, tool descriptors) instead of handing the model everything.
5. Autonomy is gated: external work flows through task intents, `curate` review, approved **Docket** tasks, and sandboxed `work` execution with credential injection at the egress gateway.

## Consolidated views

| Topic | Document |
|-------|----------|
| **Overall architecture** — components, the turn loop, storage boundary, edges | [ARCHITECTURE.md](ARCHITECTURE.md) |
| **Model experience** — what the model sees, tools, memory visibility, budgets | [MODEL_EXPERIENCE.md](MODEL_EXPERIENCE.md) |
| **Safety via stances** — trust boundaries, governance, approvals, sandboxing | [SAFETY.md](SAFETY.md) |
| **Bear portability** — moving a Bear between Den servers | [PORTABILITY.md](PORTABILITY.md) |

These are summaries with pointers; the canonical docs live under [docs/](docs/README.md).

## Start here

| If you want to… | Open |
|-----------------|------|
| **Deploy** (Coolify; **recommended:** root Docker Compose `bears-*` app stack) | [docs/guides/deployment/deployment.md](docs/guides/deployment/deployment.md), [docker-compose.yaml](docker-compose.yaml) |
| **Understand the architecture** | [ARCHITECTURE.md](ARCHITECTURE.md), then [docs/architecture/README.md](docs/architecture/README.md) for the full reading path |
| **Roadmap** | [docs/roadmap/PLAN.md](docs/roadmap/PLAN.md) |
| **Den runtime internals** | [docs/architecture/den-runtime.md](docs/architecture/den-runtime.md) |
| **Every doc in one place** | [docs/README.md](docs/README.md) |
| **Troubleshoot ACP/Zed/Code token issues** | [docs/guides/acp-troubleshooting.md](docs/guides/acp-troubleshooting.md) |

**Stack (high level):** **Den** runs the in-process agent loop and web/API surfaces; **Bifrost** is the OpenAI-compatible model gateway; **Garage** stores artifacts; **Postgres** stores Den control-plane state; per-Bear SQLite stores canonical Bear memory; **Outline** + Den provide Cabinet when shared knowledge is deployed.

**Quick deploy order:** use the root [`docker-compose.yaml`](docker-compose.yaml) for Den + Bifrost (+ optional bundled Postgres) on one network; add **`COMPOSE_PROFILES=bundled`** if you want the compose-bundled Postgres instead of a managed database. Details: [deployment guide](docs/guides/deployment/deployment.md).

---

**Coding agents & repo conventions** (GitOps, migrations, terminology, link rules): **[AGENTS.md](AGENTS.md)** — kept separate so this README stays short for humans.

## License

Copyright © The Artificial Creative B.V., registered in the Netherlands
under Chamber of Commerce (KvK) number 61953679. Source-available under
the [PolyForm Noncommercial License 1.0.0](LICENSE.md) — free to use,
modify, and share for noncommercial purposes. **Commercial use requires
a separate license**: contact <hans@theartificial.nl>.

Contributions are welcome and require agreeing to the
[Contributor License Agreement](CLA.md) — see [CONTRIBUTING.md](CONTRIBUTING.md).
