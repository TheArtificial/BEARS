# How Den Builds Bears

Den does not merely send prompts to a model. It assembles a Bear from several connected concerns: visible behavior, durable identity, current context, memory, tools, collaboration, governance, and runtime infrastructure.

This document is the beginning of a hierarchical explanation. The first and last sections are framing sections; the middle sections are intended to expand into deeper documentation over time.

## 1. Surface: Conversation and Work

This is what people directly experience when they interact with a Bear: conversation, judgment, collaboration, and useful work products. At this level, the Bear appears as a helpful counterpart that can answer, ask, plan, edit, summarize, decide, and carry work forward.

The surface is where the value is felt, but it is not where the Bear is built. The visible experience depends on the deeper systems below.

## 2. Identity: Charter and Role

What gives the Bear a durable sense of purpose.

- **Charter** — the Bear's durable responsibility boundary: what it exists to care about.
- **Role** — the mode of work it is performing: talk, pair, curate, work, watch, etc.
- **Operating style** — tone, habits, collaboration preferences, and expected behavior.
- **Responsibility boundaries** — what the Bear should own, avoid, escalate, or ask about.
- **Human relationship** — who the Bear is serving or collaborating with in a given session.

## 3. Context: Current Situation

What the Bear knows right now.

- **Active request** — the user's immediate goal, question, or task.
- **Session state** — who is present, what conversation is active, and what has happened recently.
- **Workspace state** — files, projects, tools, services, or external systems currently relevant.
- **Task frame** — constraints, deadlines, permissions, assumptions, and success criteria.
- **Relevant surroundings** — selected memory, active Mission, current domain, or open workstream.

## 4. Memory: Continuity

How the Bear carries knowledge forward.

- **Core memory** — canonical shared knowledge the Bear should retain across roles.
- **Role-local memory** — memory specific to talk, pair, curate, work, watch, etc.
- **Domains** — Bear-specific knowledge areas under that Bear.
- **Derived indexes** — searchable semantic views over canonical memory, not the source of truth.
- **Memory lifecycle** — capture, review, promotion, correction, pruning, and archival.

## 5. Action: Tools and Permissions

How the Bear can safely do things.

- **Tool catalog** — model-facing capabilities such as memory search, file edit, web fetch, etc.
- **Permission model** — what the Bear may do automatically, request approval for, or never do.
- **Adapters** — the bridge between Bear-facing tools and underlying systems.
- **Execution loop** — the Bear decides, calls a tool, observes the result, and continues.
- **Safety boundaries** — validation, scoped access, confirmations, and auditability.

## 6. Coordination: Missions and Relationships

How Bears work with people, teams, and each other.

- **Cabinet Missions** — shared work or knowledge containers involving humans and Bears.
- **Bear participation** — Bears can join Missions without owning them.
- **Shared context** — Mission history, artifacts, goals, and constraints.
- **Handoffs** — one Bear can leave useful context for another Bear or future session.
- **Collaboration patterns** — solo assistance, pair work, review, delegation, monitoring.

## 7. Governance: Reflection and Curation

How Bears improve without turning memory into a mess.

- **Reflection** — the Bear notices what may be worth remembering or improving.
- **Review requests** — role-local observations can be submitted for curation.
- **Curation** — selected knowledge is cleaned, organized, and promoted into better places.
- **Human override** — people can correct, delete, approve, or reshape what the system believes.
- **Provenance** — important memory and decisions should remain explainable.

## 8. Infrastructure: Den Runtime

This is the substrate that makes the Bear possible. Den authenticates the human, assembles the session, selects relevant context, routes tools, stores durable state, exposes APIs, and observes the runtime.

The infrastructure should usually stay backstage in the marketing story. It matters because it makes the Bear safe, durable, and explainable, but the public explanation should lead with the Bear people experience rather than the machinery that serves it.
