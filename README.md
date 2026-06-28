# Bear Den — Basic Environment for Agent Runtimes Stack

**Bear Den** is the product name. Each product assistant is a **bear**: one logical assistant with Den-owned runtime, memory, and control-plane state. **Den** (Rust, in `services/den/`) owns provisioning, users↔bears membership, first-party web chat, BearWire/ACP armature sessions, per-Bear SQLite memory, and Cabinet integration when Outline is deployed.

This repo is a **light monorepo**: `docs/`, `services/den/` for Den, `services/bifrost/` for the model gateway config, `services/garage/` for object storage, app/client packaging, and supporting service assets under `services/`.

## Start here

| If you want to… | Open |
|-----------------|------|
| **Deploy** (Coolify; **recommended:** root Docker Compose `bears-*` app stack) | [docs/guides/deployment/deployment.md](docs/guides/deployment/deployment.md), [docker-compose.yaml](docker-compose.yaml) |
| **Roadmap & architecture** | [docs/roadmap/PLAN.md](docs/roadmap/PLAN.md), [docs/architecture/overview.md](docs/architecture/overview.md) |
| **Den-native runtime architecture** | [docs/architecture/den-native-runtime.md](docs/architecture/den-native-runtime.md) |
| **Every doc in one place** | [docs/README.md](docs/README.md) |
| **Troubleshoot ACP/Zed/Code token issues** | [docs/guides/acp-troubleshooting.md](docs/guides/acp-troubleshooting.md) |

**Stack (high level):** **Den** runs the native in-process agent loop and web/API surfaces; **Bifrost** is the OpenAI-compatible model gateway; **Garage** stores artifacts; **Postgres** stores Den control-plane state; per-Bear SQLite stores canonical Bear memory; **Outline** + Den provide Cabinet when shared knowledge is deployed.

**Quick deploy order:** use the root [`docker-compose.yaml`](docker-compose.yaml) for Den + Bifrost (+ optional bundled Postgres) on one network; add **`COMPOSE_PROFILES=bundled`** if you want the compose-bundled Postgres instead of a managed database. Details: [deployment guide](docs/guides/deployment/deployment.md).

---

**Coding agents & repo conventions** (GitOps, migrations, terminology, link rules): **[AGENTS.md](AGENTS.md)** — kept separate so this README stays short for humans.

*Assistant-oriented notes also live under [.kilocode/memory_bank/](.kilocode/memory_bank/).*

## License

Add a `LICENSE` at the repo root when you publish or distribute this configuration.
