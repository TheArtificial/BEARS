# Documentation

Index of agent- and contributor-oriented docs for **this project**.

## Architecture

| Topic | Document |
|--------|-----------|
| Den runtime | [`../architecture/den-runtime.md`](../architecture/den-runtime.md) |
| Context compilation scenarios | [`../architecture/context-compilation-scenarios.md`](../architecture/context-compilation-scenarios.md) |
| Prompt fragment registry | [`../architecture/prompt-fragment-registry.md`](../architecture/prompt-fragment-registry.md) |
| Memory model | [`../architecture/memory-model.md`](../architecture/memory-model.md) |
| Den concepts overview | [`../architecture/den-concepts-overview.md`](../architecture/den-concepts-overview.md) |
| Den runtime plan | [`../roadmap/DEN_NATIVE_RUNTIME_PLAN.md`](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md) |
| Prompt fragment registry plan | [`../roadmap/PROMPT_FRAGMENT_REGISTRY_IMPLEMENTATION_PLAN.md`](../roadmap/PROMPT_FRAGMENT_REGISTRY_IMPLEMENTATION_PLAN.md) |

## Foundations

- **[`development-principles.md`](development-principles.md)** — Values and defaults: dependencies, frontend minimalism, how much to grow the stack.

## Getting started

- **[`den-quickstart.md`](den-quickstart.md)** — Local development: `.env`, migrations, `cargo run`, dev-only quirks (URL prefix, templates, mail).

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
| Stances, channels, trust | [`../architecture/bear-stances.md`](../architecture/bear-stances.md) |
| Trust, armature, stances | [`../architecture/interactive-stances-and-role-axes.md`](../architecture/interactive-stances-and-role-axes.md) |
| Work surfaces ↔ conversations | [`work-surfaces-and-conversations.md`](work-surfaces-and-conversations.md) |
| Bear memory (concise) | [`bear-memory.md`](bear-memory.md) |
| Bear package (export/import) | [`bear-package.md`](bear-package.md) |

## Operations & deploy

| Topic | Document |
|--------|-----------|
| Infra, env, logging | [`infrastructure-and-ops.md`](infrastructure-and-ops.md) |
| Container / deploy notes (env table, Docker build-arg, migrations) | [`den-deploy.md`](den-deploy.md) |
| Coolify compose stack | [`deployment/deployment.md`](deployment/deployment.md) |

## Renaming the starter

- **[`rename-from-starter.md`](rename-from-starter.md)** — checklist and greps when moving off the `newapp` placeholders.

## Plans (Bear Den / Den)

Product roadmap and Phase 1 decisions: **[`../roadmap/`](../roadmap/)** in the monorepo. [`../../services/den/plans/README.md`](../../services/den/plans/README.md) links there from inside `services/den/`.
