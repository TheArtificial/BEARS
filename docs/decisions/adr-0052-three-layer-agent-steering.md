# ADR-0052: Three-Layer Agent Steering

**Status:** Proposed  
**Date:** 2026-07-08  
**Deciders:** Hans

**Related:**

- [ADR-0050: Agent loop control, adaptive budgets, and runtime checkpoints](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)
- [ADR-0045: Session task lists as Docket checkouts and working projections](adr-0045-session-task-lists-and-docket-checkout.md)
- [ADR-0034: Jobs and Tasks Work-Management Model](adr-0034-jobs-and-tasks-work-management.md)
- [ADR-0039: Trust profiles and governance](adr-0039-trust-profiles-and-governance.md)
- [ADR-0047: Context window budget and token estimation](adr-0047-context-window-budget-and-token-estimation.md)
- [ADR-0051: Reflection performance assessments](adr-0051-reflection-performance-assessments.md)
- [MODEL_EXPERIENCE.md](../../MODEL_EXPERIENCE.md)

## Context

Bear Den needs models to keep working when useful work remains, stop or ask for help when work is blocked or complete, and avoid tool churn when the loop is no longer productive.

Simple harness steering patterns — "think out loud," "explain every N turns," "make a plan," or "don't use too many tools" — are helpful in single-user coding agents, but they are not sufficient for Bear Den because Bear runs cross more boundaries:

- a Bear may operate in `chat`, `pair`, `work`, `curate`, or `watch` stance;
- `work` may execute Docket-backed jobs with durable completion criteria;
- session task lists are working projections, not canonical task truth;
- BearWire/ACP adds client obligations and permission handshakes;
- canonical conversation history, model replay, provider reasoning, runtime progress, checkpoint artifacts, Docket events, and memory are separate projections;
- context budget and compaction can end or reshape a run;
- reflection/assessment later evaluates loop behavior from recorded evidence.

A single steering mechanism called "checkpointing" has started to absorb too many responsibilities: staying on task, preventing repeated tool loops, producing human-visible progress, creating audit records, seeding compaction, raising thinking effort, and informing Reflection. That coupling risks over-engineering the live loop and frustrating capable models that are doing reasonable work.

We need a clearer steering model that separates:

1. non-negotiable runtime safety;
2. advisory steering and legibility;
3. objective grounding and longitudinal assessment.

## Decision

Bear Den will model agent steering as three layers:

```text
Hard guards      →  Soft steering      →  Grounding and assessment
(runtime safety)    (runtime guidance)     (evidence and tuning)
```

These layers cooperate but must not collapse into one another.

### 1. Hard guards are deterministic and authoritative

Hard guards decide when a run must stop, pause, or reject a terminal response regardless of what the model says.

Hard guards include:

- unresolved client obligations blocking continuation;
- permission waits and approval state;
- wall-clock limits;
- total tool-call fuses;
- tool-class budgets;
- consecutive tool-failure limits;
- rule-of-ko / repeated same-signature limits;
- emergency hard-step fuse;
- task-list/Docket continuation gates;
- context-window overflow handling and compaction boundaries.

Hard guards are runtime-owned typed policy. They must not be implemented by prompt wording, transcript string matching, or edge-specific heuristics. They may be configured by a resolved profile, but once resolved they are authoritative for the run.

Hard guards answer questions such as:

- Is the run allowed to continue?
- Is a client/tool/permission obligation still open?
- Has the model repeated the same failed or same-position action too often?
- Is there incomplete actionable task-list/Docket work that makes a final answer invalid?
- Is the request too close to or over context limits?

A model checkpoint report cannot override a hard guard. For example, `more_exploration_justified: true` does not buy more total tool budget or reset ko. A checkpoint can only inform soft steering and assessment.

### 2. Soft steering is advisory, sparse, and typed

Soft steering helps the model choose a better next action when the hard guards indicate possible drift but have not necessarily stopped the run.

Soft steering includes:

- runtime checkpoint nudges;
- the runtime-owned `checkpoint` tool;
- checkpoint-turn thinking/reasoning effort escalation;
- low-budget warnings;
- task-focus nudges;
- run progress messages;
- provider reasoning/thought display;
- model-visible hidden continuity notes after operational outcomes.

Soft steering is intentionally lighter than hard guards:

- it guides but does not itself mutate task state;
- it may open a fresh checkpoint-observation window after the model responds;
- it is not a task event, Docket event, or memory record;
- it should degrade gracefully when malformed;
- it should be sparse enough not to frustrate capable models.

The primary structured path for checkpoints is the `checkpoint` tool, not assistant-text JSON. The checkpoint tool should structure only the few fields the runtime branches on or audits:

- `checkpoint_id`;
- active objective/task item;
- short free-text `summary`;
- `more_exploration_justified`;
- `next_action`;
- optional `task_state_change_needed` intent;
- `evidence_refs`.

Do not force the model's whole reasoning into required JSON. Structure the decision, not the thinking.

A checkpoint report is advisory/audit evidence. It may say that task state should change, but actual task state still changes only through task-management tools such as `update_task_list`, `sync_task_list`, or `request_task_list_handoff`.

After a valid or degraded checkpoint report, Den may reset checkpoint-trigger observation counters such as read/search-since-mutation, same-signature checkpoint counters, and consecutive-failure checkpoint counters. This gives the model a bounded fresh chance to read, search, or recover after synthesizing. It must not reset hard budgets, wall-clock limits, total tool-call fuses, rule-of-ko, or task gates.

Soft steering answers questions such as:

- Should the model pause and synthesize before doing more of the same?
- Should the next model call spend higher thinking effort?
- What does the model intend to do next?
- Does the model think task state should change, requiring a real task tool call?
- What concise summary should be available for audit or compaction?

### 3. Grounding and assessment provide evidence and tuning signals

Grounding and assessment decide whether steering is working and whether model behavior is actually productive.

Grounding includes objective, work-surface-native signals such as:

- tests, typechecks, diagnostics, linters;
- schema validation and dry runs;
- parse/open checks for changed artifacts;
- non-empty diff checks;
- Docket criteria evaluation;
- task-list/Docket state transitions with evidence;
- future surface-declared grounding probes per ADR-0050 §7c.

Assessment includes offline or background evaluation of runtime evidence, per ADR-0051:

- run assessments;
- session assessments;
- model/control-profile rollups;
- tuning proposals governed separately from assessment.

Grounding and assessment are the long-term feedback loop. They tell Den and operators whether hard guards and soft steering are appropriately tuned.

Grounding/assessment answer questions such as:

- Did the model's mutation actually change the work surface?
- Did validation pass or fail?
- Did a checkpoint help the model move from exploration to action?
- Did a model or control profile cause too many checkpoints, undercut runs, or allow churn?
- Are particular Bears or stances drifting over time?

Grounding results and assessments are evidence. They are not model-visible by default, do not mutate task state by themselves, and must not override hard guards unless converted into a governed policy change through the appropriate runtime/profile mechanism.

## Steering responsibilities by problem

### Continue when work remains

Use hard guards and task gates, not prompt encouragement alone.

- If a session task list or Docket-backed task has incomplete actionable items, progress-only final answers are invalid.
- If work is blocked, unsafe, not applicable, waived, cancelled, or permission-gated, the model must record that state through task tools with evidence.
- Work stance requires an active task/work state; without it, runtime should stop with a configuration blocker rather than chat generically.

### Avoid churn when work does not remain

Use deterministic stop conditions and task-state truth.

- If no incomplete actionable task remains, terminal responses are allowed.
- If repeated tool signatures, repeated failures, or repeated task-gate rejections show spinning, hard guards or checkpoint nudges intervene.
- If the model repeatedly fails to provide a different action or valid task-state update, the runtime stops or surfaces a blocker.

### Keep focus on a task when defined

Task focus belongs to session task lists and Docket, not to checkpoint prose.

- Checkpoints may reference the active item and suggest a task-state change.
- Only task tools/Docket service APIs update status, blockers, completion, criteria, sync state, or handoff state.
- A checkpoint report cannot satisfy a task gate by saying "done" or "blocked"; it can only recommend the follow-through tool action.

### Prevent spinning

Use layered detection:

1. hard deterministic signals: ko, failure streaks, total budgets, tool-class budgets;
2. soft steering: checkpoint after over-exploration or near-ko conditions;
3. grounding: validate whether a mutation was meaningful and whether post-mutation checks passed;
4. assessment: detect repeated patterns over runs/sessions and recommend tuning under governance.

Do not rely on model self-report to decide whether the model is spinning. A checkpoint can help the model recover, but it is not trusted as proof of productivity.

### Ensure human and assessment observability

Human observability and assessment observability are different projections.

Human-facing live observability should include:

- run progress;
- tool cards;
- approval waits;
- provider reasoning/thought display when available;
- concise checkpoint progress when it affects behavior;
- clear blockers and stop reasons.

Assessment/audit observability should include:

- budget/ko/failure ledger;
- checkpoint request/report artifacts;
- grounding probe results;
- task-gate rejections;
- task-list/Docket state changes;
- model/control-level metadata;
- run/session outcomes.

Provider reasoning streams may be displayed live as thought UI, but they are not assistant answer content, not canonical conversation history, not Docket events, and not model replay by default.

## Consequences

### Positive

- Steering becomes understandable as a layered system rather than an expanding checkpoint mechanism.
- Hard safety remains deterministic and auditable.
- Capable models get more room after checkpointing because checkpoint counters can reset without resetting hard budgets.
- Task truth remains in task-list/Docket tools rather than checkpoint prose.
- Human users see what is happening without every internal assessment becoming transcript content.
- Reflection/assessment gets structured evidence without being injected back into the live model loop.

### Negative / tradeoffs

- The runtime must maintain clear boundaries across more artifact types.
- Some behavior still needs tuning by model, stance, and task type.
- Tool-call checkpointing is more infrastructure than prompt-only steering.
- A sparse checkpoint report may feel less informative than free-form prose unless paired with good live progress/reasoning display.
- Grounding probes are surface-dependent and require additional work-surface metadata.

## Implementation guidance

- Keep the `checkpoint` tool schema thin. Prefer one `summary` field over many required introspection arrays.
- Prefer warnings, advisory tool results, or follow-up nudges over hard failure when checkpoint reports are malformed.
- Reset checkpoint-observation counters after a checkpoint report, but never reset hard budgets or ko from the checkpoint alone.
- When `task_state_change_needed` is present, require the next action to be a real task-management tool before unrelated work proceeds.
- Emit run progress for checkpoint triggers and checkpoint-thinking escalation.
- Preserve checkpoint artifacts for `work` audit, but keep them out of Docket events unless a task tool produced an actual task-state change.
- Add tests at all three layers: hard guard behavior, checkpoint/tool steering behavior, and projection/audit behavior.

## Non-goals

- No prompt-only stay-on-task policy.
- No checkpoint prose as task state.
- No provider reasoning as assistant answer content.
- No model-visible Reflection assessments by default.
- No learned tuning of hard safety floors such as emergency hard-step fuse or rule-of-ko.
- No assumption that all Bears work on code repositories.
- No unbounded "just keep trying" mode.

## Notes

ADR-0050 remains the detailed decision for the current agent loop control mechanics. ADR-0052 names the broader steering architecture and clarifies that checkpoints are one soft-steering primitive among hard guards, grounding, and assessment.
