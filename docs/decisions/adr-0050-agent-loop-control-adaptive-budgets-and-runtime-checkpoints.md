# ADR-0050: Agent Loop Control, Adaptive Budgets, and Runtime Checkpoints

**Status:** Accepted  
**Date:** 2026-07-04  
**Updated:** 2026-07-06  
**Deciders:** Hans

**Related:**

- [ADR-0006: Bear work surfaces for planning and work activity](adr-0006-bear-work-surfaces.md)
- [ADR-0032: Den context compaction architecture](adr-0032-den-context-compaction-architecture.md)
- [ADR-0035: Den-native in-process agent runtime](adr-0035-den-native-in-process-agent-runtime.md)
- [ADR-0039: Trust profiles and governance modes](adr-0039-trust-profiles-and-governance-modes.md)
- [ADR-0047: Context window budget and token estimation](adr-0047-context-window-budget-and-token-estimation.md)
- [ADR-0048: Core turn/client-obligation coordinator](adr-0048-core-turn-client-obligation-coordinator.md)
- [ADR-0045: Session task lists as Docket checkouts and working projections](adr-0045-session-task-lists-and-docket-checkout.md)
- [ADR-0037: Work sandbox, egress gateway, and upstream auth](adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md)
- [ADR-0051: Reflection performance assessments](adr-0051-reflection-performance-assessments.md)

> **2026-07-06 amendment.** Four changes were added after comparative review against OpenCode, Cursor, Letta Code, and Claude Code loop control:
> 1. **§7c** introduces surface-declared **grounding probes** so post-mutation feedback is grounded in the work surface's own validators rather than only in model self-report, without assuming every surface is a code repository.
> 2. **§7d** allows optional **cheap-model classifier signals** for the residual intent/similarity judgment calls, strictly advisory and ledgered, and deferred behind the measurement foundation.
> 3. **§11** promotes **context/token budget** (previously deferred entirely to [ADR-0047](adr-0047-context-window-budget-and-token-estimation.md)) to a first-class loop-control dimension owned by the loop controller, and defines checkpoint-then-compact sequencing.
> 4. The **Initial policy shape** now launches in **advisory mode** with a persisted budget ledger and offline replay, because Bear Den cannot fine-tune this policy without an automated measurement loop. Sequencing lives in [AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md](../roadmap/AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md).

## Context

The Den-native loop historically used a small, mostly flat `max_steps` ceiling as its primary protection against runaway tool loops. That protected infrastructure and user experience, but in practice it also strangled useful multi-step work.

The failure mode is especially visible when a model is productively exploring a codebase or recovering from a failed tool call: a single fixed step ceiling treats productive search, partial recovery, and obvious churn as the same thing.

This becomes more limiting as `work` grows into a long-running stance. We want Den to allow materially longer runs where appropriate, while still stopping models that are thrashing, repeating the same tool calls, blindly re-driving failed actions, or drifting through exploration without forming a useful next-action strategy.

Agent loop control therefore needs more than a stop rule. It also needs typed opportunities to steer recovery: low-budget warnings, task-gate nudges, and concise runtime checkpoints that force synthesis without turning checkpoint prose into task history.

## Decision

Den will replace flat per-turn step ceilings with typed **agent loop control**: profile-owned budget policy, loop-health state, ko/failure detection, control levels, and adaptive model-facing nudges. Its primary job is to track spend and progression quality, not simply count continuations.

**Agent loop control** is the runtime policy layer that decides how a tool-using model turn progresses across model calls and tool results: when to continue, checkpoint, retry, warn, stop, or require task-state reconciliation.

### 1. Agent loop control levels resolve to typed policy profiles

Agent loop control supports progressive **control levels**. A control level selects threshold defaults for budgets, checkpoint triggers, ko/failure tolerance, task-gate nudges, optional checkpoint-turn thinking-level behavior, and visibility behavior while preserving the same semantic invariants.

Initial levels:

| Level | Intended use | Checkpoint posture |
| --- | --- | --- |
| `light` | Strong tool-disciplined models on simple interactive tasks | Checkpoint mainly on repeated failures, repeated signatures, or low budget |
| `standard` | Default `pair`/`chat` coding and ordinary `work` execution | Checkpoint after moderate over-exploration, repeated failure, task-gate rejection, or low budget |
| `careful` | Weaker/less tool-disciplined models, risky edits, unfamiliar work surfaces, or autonomous work that benefits from stronger synthesis | Earlier exploration checkpoints, pre-broad-mutation checkpoint, stronger recovery nudges |
| `strict` | High-risk/destructive workflows or future governed autonomy requiring tight runtime supervision | Frequent synthesis, explicit pre-risk checkpoints, stricter gates and lower retry tolerance |

Control levels tune thresholds, triggers, and optional checkpoint-turn reasoning effort, not meaning. A `careful` checkpoint and a `light` checkpoint are both runtime scaffolding; neither mutates task state or substitutes for task-list/Docket records.

Control-level selection should mirror model selection:

1. the model registry provides a default agent-loop-control level for each model or model capability profile;
2. a Bear-level configuration may override that default for the Bear;
3. a stance-level configuration may override the Bear/model default for a specific stance such as `pair`, `chat`, or `work`;
4. task/run policy may request an escalation for risk, difficulty, or governance mode, but runtime receives a resolved typed profile rather than hardcoding model names or inferring risk from prose.

This lets a more tool-disciplined model run with lighter checkpointing by default, while a model known to over-explore or retry poorly can default to `standard` or `careful`. Bear and stance overrides preserve operator intent and local experience without scattering per-model conditionals throughout the runtime.

Control levels may also specify a **checkpoint thinking policy**. When provider/model configuration exposes a thinking level or reasoning-effort control, a checkpoint turn may optionally request a different thinking level than ordinary continuation turns. For example, `light` may keep the model's default thinking level, `standard` may request moderate reasoning for checkpoint turns, and `careful`/`strict` may request higher reasoning for checkpoint or pre-risk turns. This is a quality/cost tuning knob, not a safety boundary: loop continuation, task gates, budgets, and ko enforcement remain runtime-authoritative even if a provider ignores or lacks thinking-level controls.

### 2. Turn budgets are profile-owned typed policy and budget ledger

Each role profile owns a typed `TurnBudgetPolicy` with at least:

- `max_wall_clock_ms`
- `tool_call_limits` by tool class
- `max_consecutive_tool_failures`
- `max_same_tool_signature_repeats`
- `emergency_hard_steps`

This policy is part of the loop capability profile, not ad hoc string matching in BearWire or ACP.

The runtime also keeps a `TurnBudgetState` ledger with at least:

- turn start time
- cumulative tool-call usage by class
- consecutive failure streak
- last tool-batch signature
- ko repeat count

This ledger is not required to be monotonic for every class. Budgets that primarily detect unproductive exploration may be reset or replenished after a meaningful state-changing action, as long as the reset rule is typed runtime policy rather than ad hoc edge behavior.

### 3. Hard step ceilings remain only as emergency fuse

Den keeps a hard continuation ceiling because runaway loops are still a real infrastructure and UX risk.

But step count is not the primary budget dimension anymore. `emergency_hard_steps` is a deadman switch, not the main policy surface.

### 4. Primary budgets are wall-clock and tool-class spend

The first-class budget dimensions are:

- wall-clock time for the turn;
- total tool-call count;
- per-class tool-call quotas such as read/search/fetch/execute/write/destructive;
- consecutive tool failure streaks;
- ko-style repeated same-position retries.

This lets Den distinguish productive long work from expensive or risky churn.

### 5. Permission approvals are not themselves budget spend

Permission waits and approvals are workflow friction, not proof of runaway behavior.

Den must budget what happens after approval, not merely that an approval occurred. A run must not fail simply because it encountered many legitimate permission handshakes.

### 6. Rule of ko blocks repeated same-position retries

Den adopts a **rule-of-ko** style guard for agent loops:

- repeating the same tool signature too many times in a row is illegal;
- “same signature” means same tool plus normalized arguments;
- once the ko limit is hit, the loop stops even if hard steps remain.

This is the primary churn guard for long-running stances such as `work`.

### 6a. Task outcome gates need their own ko signal

Tool-call ko is not enough. A run can also loop at a task outcome gate: the model repeatedly attempts to final-answer, Den rejects the terminal response because an active task-list item remains actionable, Den nudges continuation, and the model repeats the same invalid final answer.

Den should track this as a typed gate-rejection loop, not infer it from transcript prose. The gate-rejection signature should include at least:

- active task-list id and version;
- next actionable item id/title;
- classified final-response kind;
- normalized assistant text hash.

Policy shape:

- first rejection: model-facing continuation nudge;
- repeated same rejection: stronger nudge requiring a different action or a task-state update such as blocked/cancelled/not-applicable with evidence;
- threshold exceeded: stop forcing continuation and surface a concise blocker for human review or task-state correction.

This preserves the value of the continuation gate without creating an obstinate self-loop. It also keeps reasoned non-action distinct from failure: if the remaining task is blocked, not applicable, waived, or permission-gated with evidence, the terminal response should be allowed by the task gate rather than counted as a rejection.

### 7. Repeated failures have their own budget

Den separately tracks consecutive failed tool batches.

If the model keeps driving failure without recovering, the loop stops even if the hard step budget is not exhausted. This distinguishes useful long investigation from blind retry behavior.

### 7a. Productive mutation can open a fresh verification window

Interactive stances such as `pair` often need a short read/search phase, then a mutative action, then another short read/search phase to verify the change. Treating all post-mutation verification reads as the same exploration burst is too punitive and incorrectly classifies productive progress as churn.

Den may therefore reset or replenish **read/search-style exploration budgets** after a successful meaningful mutative action such as a write/edit/execute/destructive step that changes the work state.

Guardrails:

- this applies only to budgets whose purpose is to detect unproductive exploration, not to all budget dimensions;
- global safety fuses such as wall-clock, total tool-call budget, consecutive-failure limits, and emergency hard-step limits remain turn-global;
- repeated-signature / ko protections may reset only when the mutative action materially changes the search state;
- failed or obvious no-op mutative actions must not earn a fresh exploration window automatically.

The goal is not to make turns unbounded. The goal is to distinguish "keeps reading without acting" from "acted and now needs a bounded verification pass."

### 7b. Adaptive runtime checkpoints guide loop recovery without becoming task state

Den may require a short structured **runtime checkpoint** when loop-health signals indicate that the model may be drifting, over-exploring, retrying failures, approaching budget pressure, or about to take a broad/high-risk action. The resolved control level determines how early and how often these triggers fire.

Checkpoint triggers may include:

- several `read`/`search` tool calls since the last meaningful mutative action;
- repeated same-tool or same-argument signatures approaching ko;
- consecutive tool failures;
- a task-gate rejection;
- a low remaining wall-clock, tool-call, or context budget;
- a broad, destructive, or otherwise risky mutative action about to be attempted.

A checkpoint is a model-facing runtime nudge whose primary response path is a runtime-owned **`checkpoint` tool call**, not assistant prose or JSON embedded in transcript text. The runtime may pair the checkpoint nudge with the control level's optional checkpoint thinking policy, such as temporarily raising reasoning effort for that model call when the provider supports it.

The `checkpoint` tool schema should structure only the fields the runtime branches on or audits:

- `checkpoint_id`;
- active objective/task-list item;
- short free-text `summary` for synthesis;
- `more_exploration_justified`;
- `next_action` as a small enum;
- optional `task_state_change_needed` intent;
- `evidence_refs`.

Do not force the model's whole reasoning into JSON arrays. Let provider reasoning/thinking and ordinary prose remain separate; structure the checkpoint decision, not the thinking process.

Checkpoints are **control-flow scaffolding, not work records**. They do not create, complete, block, waive, cancel, or sync task-list/Docket state. If a checkpoint concludes that task state should change, the model must use the appropriate task-management tool and provide evidence. Per ADR-0045, the task gate evaluates task-list/Docket state, not checkpoint prose.

Checkpoint reports are advisory/audit records. Deterministic loop signals — budgets, ko, task gates, and grounding probes — remain authoritative; a checkpoint cannot grant itself more budget or override runtime stop policy. Invalid or missing checkpoint tool arguments should normally degrade to a recovery nudge or advisory tool result, not hard-fail the turn. Reserve hard failure for emergency fuses, repeated unrecoverable protocol errors, or unsafe states.

After a checkpoint report is received, Den may reset the **checkpoint trigger observation window** that caused the nudge — for example read/search-since-mutation, consecutive-failure, and same-signature counters used only to decide when to ask for another checkpoint. This gives the model a bounded fresh chance to read/search or recover after synthesizing. It must not reset authoritative turn budgets, rule-of-ko hard stops, wall-clock limits, total tool-call fuses, or task gates.

Checkpoint thinking-level escalation must remain bounded to the checkpoint/pre-risk inference unless the resolved control profile explicitly says otherwise. It should improve synthesis quality without silently converting the whole run into a higher-cost reasoning mode.

### 7c. Grounding probes make post-mutation quality checkable per work surface

Checkpoints as defined in §7b ask the model to report an advisory decision through the `checkpoint` tool. Self-report is weak evidence: a drifting model can choose plausible `summary` text and `next_action` values. The runtime should, where the work surface allows, pair that advisory report with **grounding probes** — cheap, automatic, surface-native validators run after a mutative action, whose typed findings feed the loop controller.

This generalizes the useful idea behind editor/LSP feedback loops without assuming every Bear works on code. A Bear is "more than code," so grounding must be **work-surface dependent** (per [ADR-0006](adr-0006-bear-work-surfaces.md)) rather than a blind attempt to quality-review any artifact:

- each work-surface kind may declare a typed **grounding profile** — an ordered set of probes appropriate to that surface;
- probes are validators, not LLM quality review: they answer objective questions the surface can answer about itself;
- a surface that declares no probes degrades to §7b self-report behavior; the runtime never invents a probe for a surface that has none.

Illustrative probe families by surface kind:

| Work-surface kind | Example grounding probes |
| --- | --- |
| Repository / checkout | LSP or type diagnostics, `cargo check`/`tsc`, linters, a targeted test subset |
| Writing / document | prose/style linters, spelling, broken internal or `[[wikilink]]` references, frontmatter/schema validation, required-structure checks |
| Photo / media metadata | file still parses after write (e.g. exiftool round-trip), required fields present, values within controlled vocabularies, no corruption/checksum change |
| Structured data / config | schema validation, dry-run apply |
| Any surface (generic floor) | mutation produced a non-empty diff; the artifact still opens/parses |

Probe results are typed runtime signals, not prose. They serve two jobs:

1. **Checkpoint evidence.** A checkpoint request (§7b) may carry probe findings as `evidence_refs`, so the model synthesizes against ground truth instead of unaided introspection. This directly answers the risk that checkpoints "encourage narration instead of action."
2. **Mutation-meaningfulness arbitration for §7a.** §7a's "meaningful mutation" and "failed or obvious no-op mutative actions must not earn a fresh exploration window" are today unspecified policy judgments. A grounding probe is the natural arbiter: a passing probe over a non-empty diff marks the mutation meaningful and may open a bounded verification window; a failing probe feeds the consecutive-failure budget (§7) instead of granting a fresh window.

Guardrails:

- probes are budgeted like any other tool-class spend and must not themselves become an unbounded loop; a probe that times out or errors degrades to "no grounding signal," never to a hard failure of the turn;
- probes must not mutate the work surface;
- probe selection resolves from typed work-surface metadata, not from prose or hardcoded per-surface conditionals in the runtime;
- probe findings are runtime evidence, not task state: a failing probe does not itself block a task; only task-management tools change task state (§7b, ADR-0045).

### 7d. Optional cheap-model classifier signals (deferred)

A cheap model may act as a bounded **classifier** feeding loop-control signals, but only for decisions that are genuinely judgment calls rather than objectively computable. The useful distinction is **classification vs. evaluation**: cheap models are acceptable classifiers ("is this a loop, is this evasion, are these two calls the same intent?") and poor evaluators ("is this output correct, is this the right approach?"). Deterministic signals (§2–§7) and grounding probes (§7c) already cover everything objectively computable; a classifier is worth introducing only for the residual intent/similarity classification the typed signals cannot make.

Candidate classifier signals:

- **task-gate intent (§6a):** distinguishing a legitimate blocked/not-applicable/waived terminal response from evasion of an actionable item, which the gate-rejection signature alone cannot tell apart;
- **semantic ko:** near-duplicate churn that normalized-signature ko (§6) misses because arguments were reworded rather than repeated;
- **no-probe grounding fallback (§7c):** a coherence check on surfaces that declare no objective probe, which is explicitly the degraded path;
- **checkpoint adherence (§7b):** whether a checkpoint tool report commits to a genuinely different next action or merely narrates.

Two constraints keep this safe rather than reckless, and both are load-bearing:

1. **Advisory, never authoritative.** A classifier signal may raise a checkpoint or feed a would-stop diagnostic, but it must not overrule a deterministic budget/ko/failure decision or veto the strong model's action. A weaker model overseeing a stronger one on a hard task is inverted competence; it is exactly the component most likely to misjudge whether the primary model is thrashing or working. Classifier output is one more signal source, subordinate to the same advisory-mode discipline as everything else (§Initial policy shape).
2. **Verdicts are ledgered.** A classifier verdict that influences a decision must be persisted as a typed signal in the budget ledger, so offline replay reads the recorded verdict instead of re-invoking the model. This preserves the zero-model-call replay property (§Initial policy shape) that the tuning loop depends on. A classifier signal that cannot be recorded and replayed must not gate behavior.

Classifier invocation should be bounded to trigger boundaries (a checkpoint firing, a gate rejection, a no-probe mutation), never run per step, and must never be pointed at correctness of the primary model's work — that is the objective probes' job (§7c), not an LLM "is this good?" review. This capability is **deferred behind the advisory + replay foundation** and is not part of the first implementation slice.

### 8. Work gets a larger total budget than interactive pair/chat

`work` is expected to support materially longer runs than `pair`, `chat`, or `watch`.

The runtime therefore must support materially larger wall-clock budgets, total tool-call budgets, and class-specific budgets for `work`, while still applying the same ko/failure protections.

### 9. The decision is core runtime policy, not edge behavior

As with approvals and client obligations in ADR-0048, control-level resolution, continuation-budget, and checkpoint decisions are core runtime semantics.

BearWire and ACP may project the resulting warning, checkpoint request, or stop reason, but they do not decide whether the model is allowed to continue.

### 10. Runtime outcomes and checkpoints have explicit visibility semantics

When a turn ends because of budget exhaustion, loop-ko, repeated tool failure, or operational runtime failure, Den should persist a normalized **model-visible but user-hidden** transcript record.

This record exists so future turns in the same conversation do not falsely assume the previous turn completed successfully.

Requirements:

- persist a short normalized summary, not raw proxy or transport garbage;
- keep the row visible to model transcript replay;
- keep the row hidden from ordinary user-facing conversation history;
- include typed metadata such as reason, retryability, subsystem, and run id.

The model should learn that prior work may have partially succeeded and that it should resume from the latest successful state rather than assuming completion.

The detailed operational record being hidden from ordinary history does not mean the human should see nothing. Per [Runtime Error UX Policy](../architecture/runtime-error-ux-policy.md), Den should also project a concise user-visible chat/history marker whenever model-visible budget warnings, task-focus warnings, recovery notes, or operational outcomes affect model behavior.

The marker should explain the behavioral change in product language, while the hidden record retains the detailed model-continuity instruction and structured diagnostics.

Runtime checkpoints may be projected as live/user-ephemeral run progress or persisted as model-visible hidden continuity records when useful for replay. They must not be projected as Docket job/task events unless an explicit task-management tool call created a corresponding task-state change.

### 11. Context/token budget is a first-class loop-control dimension

The original decision deferred token-aware continuation budgets entirely to [ADR-0047](adr-0047-context-window-budget-and-token-estimation.md) as a "compatible future extension." That split is wrong for long `work` runs: context pressure is the dimension most likely to end a productive run, and the loop controller already lists "low remaining context budget" as a checkpoint trigger (§7b). Treating context as an external signal imported after the fact means the controller cannot coordinate its two most important end-of-turn behaviors — checkpointing and compaction.

Den therefore promotes **context/token budget to a first-class loop-control dimension**, alongside wall-clock and tool-class spend (§4):

- ADR-0047 remains the authority for *how* context is measured (per-component estimation on the assembled request, calibration against provider usage, precision labeling). This ADR does not re-specify tokenization.
- The loop controller *consumes* the ADR-0047 budget report as a live dimension of `TurnBudgetState`, so remaining-context thresholds are evaluated in the same place as ko, failure-streak, and wall-clock thresholds rather than in a separate compaction path.
- Control levels tune context thresholds like any other budget: `careful`/`strict` may checkpoint earlier under context pressure than `light`.

**Checkpoint-then-compact sequencing.** When context pressure crosses the checkpoint threshold, the controller should prefer to run a checkpoint (§7b) *before* compaction, then use the checkpoint tool report as a high-fidelity compaction seed:

- the model-authored checkpoint report (active objective, summary, evidence refs, next action) is produced at exactly the moment a good summary is needed;
- compaction (per [ADR-0032](adr-0032-den-context-compaction-architecture.md)) can then use that concise checkpoint report as a seed instead of summarizing mid-thrash from generic instructions;
- if context pressure is already critical — no room to afford a checkpoint turn — compaction runs first and the checkpoint is deferred, so the ordering is a preference, not an invariant that can itself exhaust the window.

Compaction remains subordinate to the same boundaries as checkpoints: it is control-flow scaffolding, does not mutate task-list/Docket state, and does not turn checkpoint tool reports or prose fallback into task history.

## Consequences

### Positive

- Productive multi-tool turns are budgeted by actual spend and loop health instead of by a small flat continuation count.
- `work` can support longer runs without disabling loop safety.
- Repeated same-call churn is blocked explicitly instead of being indirectly caught only by a coarse step limit.
- Failure loops are treated separately from exploratory progress.
- Adaptive checkpoints can force concise synthesis before more exploration or risky mutation.
- Control levels let Den tune checkpoint intensity and optional checkpoint-turn thinking level for different models, Bears, stances, task risks, and governance modes without changing checkpoint semantics.
- The policy remains typed and role-owned.
- Interactive verification after a real mutation is less likely to be cut off as false-positive read churn.
- Checkpoints remain subordinate to task-list/Docket state instead of becoming informal task records.
- Grounding probes (§7c) let checkpoints synthesize against surface-native ground truth and give §7a's "meaningful mutation" judgment an objective arbiter, while degrading cleanly on surfaces that declare no probes.
- Context/token budget (§11) is evaluated in the same controller as ko, failure, and wall-clock, and checkpoint-then-compact turns a forced synthesis into a high-fidelity compaction seed.
- Advisory-first rollout with a persisted ledger and offline replay lets a single-maintainer project tune thresholds against recorded runs at near-zero cost, without a live eval platform.
- Optional cheap-model classifier signals (§7d) can fill the residual intent/similarity judgment calls (gate evasion, semantic ko, no-probe coherence, checkpoint adherence) without displacing the deterministic and objective signals that remain authoritative.

### Negative / tradeoffs

- The loop controller now carries more state.
- Tool-class quotas, checkpoint triggers, and control-level defaults are policy choices that will need tuning with real usage.
- Some borderline cases will still stop early or late until the policy evolves with more signals.
- Poorly worded checkpoints could encourage narration instead of action; prompts should require concise state, evidence, and next action rather than open-ended reasoning.
- Model-associated defaults risk hiding product behavior in model configuration unless the resolved level is observable in run diagnostics.
- Checkpoint-turn thinking escalation can increase latency/cost, and provider support may be uneven; it must be treated as best-effort model configuration rather than a guarantee.
- Grounding probes add per-surface maintenance: each work-surface kind needs a declared, maintained probe set, and probes cost spend and latency of their own; a slow or flaky probe must degrade to "no grounding signal" rather than stalling or failing the turn.
- Promoting context budget into the loop controller couples it to ADR-0047's estimation accuracy; a mis-estimated budget now influences checkpoint/compaction timing directly, so precision labeling and calibration matter more.
- A cheap-model classifier (§7d) adds a nondeterministic, latency- and cost-bearing signal source and risks inverted competence if ever made authoritative; it is safe only while advisory, ledgered, bounded to trigger points, and kept away from correctness evaluation.
- The policy is deliberately complex and performs best fine-tuned, yet Bear Den is a personal project: without the advisory-mode ledger and replay harness there is no feedback loop to justify the complexity, so that measurement machinery is a prerequisite, not an optional nicety.

## Initial policy shape

Because the policy performs best when tuned and Bear Den has no live evaluation platform, the first implementation deliberately **launches in advisory mode with a narrow tunable surface** and earns additional dimensions from recorded evidence rather than shipping the fully-tuned end state on day one.

The first implementation should use:

- **advisory (observe/log-only) mode by default.** The controller resolves the profile, maintains the full budget ledger, and evaluates every trigger, but only ko and the emergency hard-step fuse actually stop the loop. All other triggers emit `would_*` diagnostics without changing behavior until real-usage rates justify enforcement;
- a **persisted per-turn budget ledger** (tool signatures, tool classes, timestamps, failure flags, gate-rejection signatures, context-budget snapshots — no transcript content required), so any recorded turn can be **replayed offline against alternative control profiles with zero model calls**. This replay harness is the project's tuning loop;
- **two launch levels, not four:** `standard` and `careful`, with generous thresholds. `light` and `strict` are defined but deferred until replay evidence shows they behave better than a well-chosen `standard`/`careful`;
- model-registry defaults plus Bear-level and stance-level overrides that resolve before turn execution;
- a resolved profile-owned wall-clock budget, total and per-class tool-call budgets, consecutive-failure cutoff, ko-style repeated-signature cutoff, and ko-style repeated task-gate-rejection cutoff;
- context/token budget consumed from ADR-0047 as a first-class ledger dimension (§11);
- checkpoint triggers for over-exploration, repeated failure, task-gate rejection, and low remaining wall-clock/tool/context budget, evaluated in advisory mode first;
- grounding probes (§7c) for at least the repository work-surface kind and the generic non-empty-diff floor, with other surface kinds added incrementally;
- optional checkpoint thinking policy per control level, kept off at launch and enabled per level once checkpoint enforcement is on;
- a high emergency hard-step fuse.

Cheap outcome labeling should accompany the ledger so replay has a signal to optimize against: a stop immediately followed by the user re-asking or saying "continue" is a probable false positive; a normal end after a long read-only tail with no mutation is a probable false negative. Heuristic labels over the run log give a tuning trend line without human annotation.

Enforcement, additional levels, per-class thinking escalation, and additional grounding surfaces are unlocked from this advisory baseline as evidence accrues, per [AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md](../roadmap/AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md). Because Bear Den's userbase is small and non-technical, the party that reads this ledger and learns per-model tuning is not a human maintainer but Reflection's `curate` role, via the `observe → assess → propose → apply` pipeline in [ADR-0051](adr-0051-reflection-performance-assessments.md); loop-control assessments produced there must never be made model-visible or allowed to tune the hard safety floor (rule-of-ko, emergency fuse).

## Implementation notes

- `pair` and `chat` should keep moderate wall-clock and tool-call budgets.
- `work` should receive significantly higher wall-clock and tool-call budgets than interactive stances.
- The runtime should log or expose the resolved agent-loop-control level, its source (`model_default`, `bear_override`, `stance_override`, or `task_escalation`), and any checkpoint thinking-level override applied to a model call for diagnostics.
- The emergency step fuse should be high enough that it only catches pathological loops or missing health signals.
- Model-visible low-budget warnings and runtime checkpoints are desirable, but runtime enforcement still remains authoritative.
- Normalized operational outcome records should be persisted for future transcript replay whenever a run/turn fails after work has already been attempted.
- If `pair` or `chat` replenish read/search budget after a successful mutative step, the replenishment should be small and verification-oriented rather than a full license to restart the turn.
- Checkpoint reports should be delivered through the runtime-owned `checkpoint` tool whenever possible. Assistant-text JSON is only a degraded compatibility fallback and must not be the primary path.
- Checkpoint content should be compact: active objective, a short synthesis summary, decision fields, evidence refs, and any required task-state update intent.
- Checkpoints should reference task-list/Docket identifiers and versions when relevant, but task-state changes must still go through task-management tools.
- Runtime code should depend on resolved control profiles and typed capability metadata, not on scattered string comparisons against provider or model names.
- Thinking-level controls should be modeled as provider/model request metadata, separate from loop-control enforcement state, so unsupported providers can degrade gracefully.
- Grounding profiles should resolve from work-surface metadata (ADR-0006) the same way control levels resolve from model/Bear/stance config; a surface with no declared profile uses only the generic non-empty-diff/parse floor. Probe execution should reuse existing tool-class budgeting and time out into a "no signal" result rather than failing the turn.
- The context-budget dimension should be read from the ADR-0047 budget report object rather than recomputed; the loop controller stores the latest snapshot in `TurnBudgetState` and compares against level-tuned thresholds. Checkpoint-then-compact ordering should be a preference the controller can abandon when the window is already too tight to afford a checkpoint turn.
- The budget ledger should be persisted in a form replayable offline against alternative profiles without any model calls; keep transcript content out of it so replay stays cheap and privacy-preserving. Advisory (`observe`) vs `enforce` should be a runtime flag so enforcement is switched on per trigger class as evidence accrues.

## Non-goals

- No unbounded “just keep trying” loop mode.
- No ACP-only or BearWire-only loop heuristics.
- No free-form transcript string matching to detect loops.
- No planner-only execution gate as a substitute for core continuation policy.
- No use of checkpoints as a substitute for task-list/Docket updates, completion criteria, or history-visible task records.
- No requirement for verbose chain-of-thought or open-ended self-explanation; checkpoints should be concise runtime synthesis through the `checkpoint` tool.
- No hardcoded per-model loop behavior in the agent runtime; model association belongs in registry/configuration and resolves to typed control profiles before execution.
- No reliance on thinking-level escalation as a substitute for budget, ko, checkpoint, or task-gate enforcement.
- No grounding probe that mutates the work surface, performs LLM "is this good?" review, or blocks a task by itself; probes are objective validators whose findings are runtime evidence, and absent a declared profile the surface simply has no grounding signal.
- No re-implementation of context/token estimation in the loop controller; §11 consumes the ADR-0047 budget report and only governs how the loop reacts to it.
- No enforcement of adaptive triggers before advisory-mode replay evidence supports the specific trigger class; ko and the emergency hard-step fuse remain the only always-on stops.
- No authoritative cheap-model overseer: classifier signals (§7d) may advise and feed the ledger but must never overrule deterministic budget/ko/failure decisions, veto the primary model's action, run per step, gate behavior without a recorded replayable verdict, or evaluate correctness of the primary model's work.
