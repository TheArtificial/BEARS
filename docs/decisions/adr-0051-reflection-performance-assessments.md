# ADR-0051: Reflection Performance Assessments

**Status:** Accepted  
**Date:** 2026-07-06  
**Deciders:** Hans

**Related:**

- [ADR-0018: Reflection system](adr-0018-reflection-system.md)
- [ADR-0050: Agent loop control, adaptive budgets, and runtime checkpoints](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)
- [ADR-0039: Trust profiles and governance modes](adr-0039-trust-profiles-and-governance-modes.md)
- [ADR-0036: Bear profile registry](adr-0036-bear-profile-registry.md)
- [ADR-0045: Session task lists as Docket checkouts and working projections](adr-0045-session-task-lists-and-docket-checkout.md)
- [AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md](../roadmap/AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md)

## Context

[ADR-0018](adr-0018-reflection-system.md) established Reflection as the umbrella system for background review, learning, and bounded improvement, with an `introspection` lane defined as "review role behavior, tool-use issues, failures, and patterns" owned by `curate`. It also fixed the governing invariant: *Reflection may learn and recommend freely, but behavior-changing adaptation is governed.*

What ADR-0018 did not do is name the **output** of introspection. It described the flow — observe evidence, create a proposal, review, apply — but treated evaluation and proposal as one motion. Two developments make that gap worth closing:

1. [ADR-0050](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md) introduces a per-turn budget ledger and offline replay whose explicit purpose is to tune loop dynamics per model. Its tuning story assumed a human maintainer reading ledgers. Bear Den's userbase is small and non-technical; that maintainer-driven loop does not scale to them, and `curate`/Reflection is the native mechanism for maintenance a user should not have to perform.
2. Detecting that a Bear is performing poorly has value **independent of any mechanism to improve it** — especially as longitudinal data. "This model needed more re-asks this month," "this Bear's runs churn more since a model version bump," and "runs under this profile stop productive work too early" are useful findings even when nothing is changed automatically in response.

Conflating *evaluation* with *proposing change* forces the higher-risk, behavior-adjacent concern to gate the lower-risk, immediately-useful one. Separating them lets Bear Den ship evaluation first, accrue longitudinal signal, and only later — under governance — let that signal steer behavior.

## Decision

Reflection evaluation is modeled as an explicit four-stage pipeline, and its evaluative output is a first-class typed artifact called an **assessment**.

```text
observe  →  assess  →  propose  →  apply
(runtime)  (Reflection) (Reflection)  (governed)
```

- **observe** is the runtime emitting evidence during a run (e.g. the ADR-0050 ledger, checkpoints, continuity records, task/tool outcomes). It is not Reflection.
- **assess** is Reflection reading that evidence and producing an **assessment**: an evaluation of Bear/role performance over a defined scope. Read-only and behavior-neutral.
- **propose** is Reflection turning one or more assessments into a **tuning proposal**: a recommended, behavior-adjacent change that cites the assessments justifying it.
- **apply** is enacting an accepted proposal, under the governance ADR-0018 already requires for behavior-changing adaptation.

### 1. Assessment is the named output of the introspection lane

The `introspection` lane (ADR-0018) produces typed assessments. An assessment is an evaluative record, not a proposal and not a change. Producing an assessment is a Low-risk Reflection action ("add an observation," "detect repeated patterns") per ADR-0018's risk boundaries.

Assessments are general: this ADR is written because agent loop control (ADR-0050) is the first consumer, but an assessment evaluates role/Bear performance and is not limited to loop dynamics.

### 2. Assess and propose are separable, and assess has standalone value

Assessment does not require a proposal or apply mechanism to exist. Assessments are valuable on their own as operator-facing and longitudinal data: regression detection across model versions, drift detection for a Bear, and evidence for *manual* tuning.

Assessment is also a strict prerequisite for proposal: no tuning proposal may be created without citing the assessments that justify it. An assessment-only implementation is therefore never throwaway work.

### 3. Assessment scopes are `run`, `session`, and `model`

- **run assessment** — evaluates one run/turn. Mechanical and cheap: churn/ko proximity, grounding-probe pass density, spend-vs-outcome, exploration-without-mutation. High volume.
- **session assessment** — evaluates a conversation/session across multiple runs. Experiential: repeated re-asks, quality drift, recovery vs spiral. This is the scope closest to the human's felt experience and is the primary longitudinal signal for a non-technical userbase.
- **model assessment** — a **derived rollup** of run/session assessments per model (and profile-hash), not a fourth primary artifact. This is where "learn how to tune this model" lives.

Run and session assessments are the atomic records; the model performance profile is a view over them.

### 4. Assessments are Reflection/operator-facing, not model-visible by default

This is a hard boundary, mirroring the visibility discipline of [ADR-0050 §10](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md).

Runtime checkpoints and continuity records exist *to inform the next turn* and are deliberately model-visible. An assessment is a report card *about* the Bear. It must not be injected into the model's context by default, must not become task-list/Docket state (per ADR-0045), and must not appear in ordinary conversation history.

Feeding a Bear its own assessment invites gaming and self-fulfilling behavior, and it is not the thing that changes runtime behavior — only an applied proposal is. If a future decision wants a role to see an assessment, that must be a deliberate, separate, scoped choice, not the default.

### 5. Assessments are derived and reproducible, not opaque judgments

An assessment must be reconstructable from recorded evidence (primarily the ADR-0050 ledger), so it is auditable and replayable rather than an unaccountable verdict. Assessments should be built from objective/typed signals where available. Subjective signals (e.g. a cheap-model classifier per ADR-0050 §7d) are permitted but must be labeled as such and carry the producing model handle.

Every assessment record carries at least: scope, subject (Bear/role/model), the control profile-hash it was produced under, typed performance dimensions, and an explicit **confidence and sample context**.

### 6. Thin data must constrain confidence, not be hidden by it

Bear Den's small, non-technical userbase produces sparse per-model data and weak feedback signals (a non-technical user is more likely to silently accept a truncated answer than to push back, so under-cutting is systematically under-observed).

Therefore:

- model-scoped assessments must not firm up below a configured sample floor; below it they report low confidence rather than a confident conclusion;
- confidence must be explicit and honored downstream (a low-confidence assessment cannot justify a high-risk proposal);
- absence of user complaint is weak evidence, not proof of success; assessment dimensions should weight catching under-cutting at least as heavily as catching over-running.

### 7. Assessment is Low risk; behavior-changing apply remains governed

Per ADR-0018's risk boundaries:

- producing an assessment or creating a proposal is **Low risk** and may run automatically within Reflection budgets;
- **applying** a loop-dynamics/behavior change is **High risk** — it "alters autonomous behavior policy" and can "change task execution strategy" — and is gated exactly like `skill_apply`: bounded, reversible (versioned profiles), and escalated to human review for large or low-confidence deltas.

An assessment never applies anything. The separation of `assess` from `apply` is the mechanism that keeps evaluation ungoverned-but-safe while behavior change stays governed.

### 8. Loop-dynamics tuning uses two Reflection lanes

Loop-dynamics learning is realized as:

- an **assessment lane** (extends `introspection`): consume the ADR-0050 ledger/replay, emit run/session/model assessments;
- a **tuning lane** (parallel to `skill_review`/`skill_apply`): turn assessments into tuning proposals and, under governance, apply bounded reversible profile deltas.

The lanes graduate independently. The assessment lane may run long before any tuning lane exists. Neither lane may tune the hard safety floor — rule-of-ko and the emergency hard-step fuse (ADR-0050) are never subject to learned adjustment; only advisory thresholds are.

## Consequences

### Positive

- Detection/evaluation is decoupled from behavior change, so the low-risk, immediately-useful half ships first and independently.
- Longitudinal performance data becomes a first-class deliverable: model regression detection, Bear drift, and profile-quality evidence, even with no auto-tuning.
- ADR-0050's tuning loop gains its missing owner (`curate`/Reflection) without assuming a technical maintainer reads ledgers.
- The model-visibility boundary keeps assessments from corrupting the runs they measure.
- Reproducibility from the ledger keeps assessments auditable rather than opaque.
- Thin-data discipline is encoded in the artifact (confidence/sample floor) rather than left to downstream good behavior.

### Negative / tradeoffs

- Another typed artifact and two more Reflection lanes to build, schedule, budget, and audit.
- Assessment dimensions are themselves policy choices that need iteration; a bad rubric produces confident-but-wrong longitudinal data.
- Sparse per-model data limits how quickly model assessments can say anything firm; the honest result is often "not enough signal yet," which must be acceptable.
- Keeping assessments model-invisible forgoes a tempting in-run self-improvement signal; that is a deliberate cost paid for integrity.

## Non-goals

- No assessment that mutates task-list/Docket state, conversation history, or runtime behavior; assessments are evaluative only.
- No model-visible assessment by default; a Bear does not see its own report card without a separate deliberate decision.
- No auto-apply of behavior-changing tuning from an assessment; apply remains governed per ADR-0018.
- No learned adjustment of the ADR-0050 hard safety floor (rule-of-ko, emergency hard-step fuse).
- No confident per-model conclusion below the configured sample floor.
- No treatment of "no user complaint" as authoritative success.

## Implementation notes

- Reuse ADR-0018's shared Reflection run/event tables; add lane-specific assessment and proposal tables rather than a bespoke store.
- An assessment record should carry: id, scope (`run`/`session`/`model`), subject refs (Bear/role/model handle), profile-hash, typed dimension scores, confidence, sample context, evidence refs into the ledger, producing-lane and (if any) producing-model handle, and created-at.
- Model assessments should be computed as rollups over run/session assessments with an explicit sample floor and confidence function, not stored as independent primary judgments.
- Assessment production should be budgeted under Reflection like any lane; where it consumes the ADR-0050 replay harness it should prefer offline, model-call-free scoring.
- Sequencing lives in [AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md](../roadmap/AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md) Part E, which builds the assessment lane before the tuning lane.
