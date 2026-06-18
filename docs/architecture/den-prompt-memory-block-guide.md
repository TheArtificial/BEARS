# Den Prompt Memory Block Guide

This guide summarizes how prompt memory blocks should be used during the Letta migration.

## What a prompt block is

A prompt memory block is an editable piece of scoped context that Den intentionally includes in prompt assembly.

Use a prompt block when you need:

- stable context across turns,
- scoped working guidance for a role or work surface,
- or a bounded editable note that should shape future reasoning directly.

## What a prompt block is not

Do not use prompt blocks as a substitute for:

- transcript history,
- arbitrary long-form durable memory,
- semantic retrieval results,
- or rolling session compaction summaries.

## Good first use cases

- a `pair`-local block describing a repo caveat that should shape future coding turns,
- a work-surface block capturing stable architectural assumptions for one service,
- a short session focus block capturing an active temporary objective,
- a reviewed role guidance block that should remain editable and directly included.

## Avoid these mistakes

- Do not attach blocks to provider-managed identities as the conceptual source of truth.
- Do not flatten all memory into always-on prompt context.
- Do not let block edits happen without audit/provenance.
- Do not confuse prompt blocks with durable shared memory promotion.

## Prompt assembly intuition

In simple terms:

- transcript tells Den what happened,
- memory tells Den what it has learned,
- retrieval helps Den find relevant material,
- compaction helps Den keep long sessions bounded,
- prompt blocks tell Den what standing scoped context should be actively in front of the model.
