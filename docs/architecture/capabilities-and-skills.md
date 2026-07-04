# Capabilities and Skills

Capabilities describe what a Bear is allowed to do. Tools are concrete executable actions. Skills are reusable procedures or knowledge packages that help a Bear perform work consistently.

This document explains the distinction and the governance boundary between them.

## Summary

- A capability is a product-level permission or ability.
- A tool is a concrete operation exposed to one or more stances.
- A skill is reusable know-how.
- Den owns capability policy and tool exposure.
- Durable skill learning is reviewed and governed; it is not arbitrary self-installation.

## Capabilities

A capability is something a Bear can do from the human or operator perspective.

Examples:

- use a GitHub integration;
- read a repository through a trusted armature;
- create background work;
- inspect a deployment;
- write to a particular external system;
- use a team-specific engineering workflow.

Capabilities are product language. They may map to tools, credentials, policies, prompt fragments, memory structures, or sandbox rights underneath.

## Tools

A tool is a concrete action the runtime can execute or request.

Examples:

- browse memory;
- read or edit files through an armature;
- fetch documentation from the web;
- update a workboard plan;
- request a task handoff;
- write a memory entry;
- inspect a browser page;
- run a command in a sandbox.

Tool access is stance-scoped and policy-scoped. A tool that is safe for `pair` may be unsafe for `chat`; a tool that is valid for `work` may be invalid for `watch`.

## Skills

A skill is reusable know-how.

Examples:

- coding conventions;
- integration playbooks;
- recurring debugging procedures;
- review checklists;
- domain-specific interpretation guidance;
- team-specific writing or planning conventions.

Skills are not just raw local files and not just arbitrary memory snippets. They are governed reusable procedures with applicability and provenance.

## How skills are represented

There are two important classes of skill material in Bear Den.

### 1. Repository-authored skill material

Human-authored prompts, procedures, and related reusable assets live in the repository as ordinary versioned artifacts. These are part of the operator/developer-managed configuration surface.

Typical uses:

- prompt fragments
- policy text
- stance instructions
- reusable procedural content packaged for runtime projection

### 2. Bear-learned durable know-how

When a Bear learns a reusable procedure through reflection or review, the durable canonical record belongs to Bear-governed state, not to an arbitrary local runtime path.

That durable representation should carry:

- content or reference to content;
- provenance;
- review state;
- stance applicability;
- dependency or prerequisite metadata;
- and any projection/runtime materialization metadata Den needs.

In other words: skills are governed Bear knowledge with execution-oriented metadata.

## Stance applicability

Not every stance should receive every capability, tool, or skill.

| Stance | Typical capability shape |
|--------|--------------------------|
| `chat` | user-facing conversational help and lightweight retrieval |
| `pair` | trusted local collaboration and interactive planning |
| `review` / `curate` | review, curation, memory integration, approval |
| `work` | approved execution with scoped external effects |
| `watch` | inbound event interpretation and observation writing |

Stance applicability is metadata and policy, not path hierarchy.

## Skill proposals and approval

Agents do not install durable skills directly.

The normal flow is:

1. a stance or reflection lane identifies a reusable procedure or convention;
2. it proposes the skill through a Den-governed review path;
3. review/curation evaluates the proposal;
4. Den records the approved applicability, provenance, and projection metadata;
5. affected stances receive the resulting skill material through Den-owned runtime configuration.

High-risk capability changes remain human-governed when appropriate.

## What Den owns

Den owns the canonical records for:

- which capabilities a Bear has;
- which tools each stance can use;
- which skills are approved;
- which stances those skills apply to;
- and whether runtime projection matches intended policy.

## Product language

Prefer:

- “This Bear has the GitHub capability.”
- “This skill applies to `chat` and `pair`.”
- “The Bear proposed a new skill for review.”
- “Den projects tools and skills according to policy.”

Avoid:

- “Every agent can use every tool.”
- “Skills are just arbitrary local files.”
- “The Bear installed a durable skill without review.”
- “A capability is only a tool id.”

## Related docs

- [bear stances](bear-stances.md)
- [bears and den](bears-and-den.md)
- [memory model](memory-model.md)
- [reflection system](reflection-system.md)
- [tasks and autonomy](tasks-and-autonomy.md)
