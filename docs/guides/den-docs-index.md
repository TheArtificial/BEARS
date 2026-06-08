# Documentation

Index of agent- and contributor-oriented docs for **this project**.

## Foundations

- **[`concepts-overview.md`](concepts-overview.md)** — Layout: web vs API vs core, migrations, what ships in the slim template.
- **[`development-principles.md`](development-principles.md)** — Values and defaults: dependencies, frontend minimalism, how much to grow the stack.

## Getting started

- **[`quickstart.md`](quickstart.md)** — Local development: `.env`, migrations, `cargo run`, dev-only quirks (URL prefix, templates, mail).

## Stack patterns

| Topic | Document |
|--------|-----------|
| Axum (this repo: routers & layers) | [`axum-in-this-repo.md`](axum-in-this-repo.md) |
| Axum (handlers & extractors) | [`axum-handler-patterns.md`](axum-handler-patterns.md) |
| SQLx | [`sqlx-patterns.md`](sqlx-patterns.md) |
| MiniJinja contexts | [`minijinja-context-patterns.md`](minijinja-context-patterns.md) |
| MiniJinja vs Jinja2 | [`minijinja-template-limitations.md`](minijinja-template-limitations.md) |
| Frontend (templates, CSS, JS) | [`frontend-development.md`](frontend-development.md); [`deep-chat-styling.md`](deep-chat-styling.md) |

## ACP runtime

| Topic | Document |
|--------|-----------|
| Concurrency model (prompt stream ↔ tool-result POST rendezvous) | [`acp-den-concurrency-model.md`](acp-den-concurrency-model.md) |
| Runtime invariants | [`acp-runtime-invariants.md`](acp-runtime-invariants.md) |
| Lessons learned | [`acp-lessons.md`](acp-lessons.md) |
| Troubleshooting | [`acp-troubleshooting.md`](acp-troubleshooting.md) |

## Bear concepts (Den / product)

| Topic | Document |
|--------|-----------|
| Roles, channels, trust | [`../architecture/bear-roles.md`](../architecture/bear-roles.md) |
| Trust, armature, profiles | [`../architecture/interactive-profiles-and-role-axes.md`](../architecture/interactive-profiles-and-role-axes.md) |
| Work surfaces ↔ conversations | [`work-surfaces-and-conversations.md`](work-surfaces-and-conversations.md) |

## Operations & deploy

| Topic | Document |
|--------|-----------|
| Infra, env, logging | [`infrastructure-and-ops.md`](infrastructure-and-ops.md) |
| Container / deploy notes (env table, Docker build-arg, migrations) | [`deploy.md`](deploy.md) |

## Renaming the starter

- **[`rename-from-starter.md`](rename-from-starter.md)** — checklist and greps when moving off the `newapp` placeholders.

## Plans (Bear Den / Den)

Product roadmap and Phase 1 decisions: **[`docs/planning/`](../../docs/planning/)** in the monorepo. [../plans/README.md](../plans/README.md) links there from inside `services/den/`.
