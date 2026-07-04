# ADR-0050: Budget-Ledger-First Turn Budgets and Rule-of-Ko Loop Detection

**Status:** Accepted  
**Date:** 2026-07-04  
**Deciders:** Hans

**Related:**

- [ADR-0035: Den-native in-process agent runtime](adr-0035-den-native-in-process-agent-runtime.md)
- [ADR-0039: Trust profiles and governance modes](adr-0039-trust-profiles-and-governance-modes.md)
- [ADR-0047: Context window budget and token estimation](adr-0047-context-window-budget-and-token-estimation.md)
- [ADR-0048: Core turn/client-obligation coordinator](adr-0048-core-turn-client-obligation-coordinator.md)
- [ADR-0037: Work sandbox, egress gateway, and upstream auth](adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md)

## Context

The Den-native loop historically used a small, mostly flat `max_steps` ceiling as its primary protection against runaway tool loops. That protected infrastructure and user experience, but in practice it also strangled useful multi-step work.

The failure mode is especially visible when a model is productively exploring a codebase or recovering from a failed tool call: a single fixed step ceiling treats productive search, partial recovery, and obvious churn as the same thing.

This becomes more limiting as `work` grows into a long-running stance. We want Den to allow materially longer runs where appropriate, while still stopping models that are thrashing, repeating the same tool calls, or blindly re-driving failed actions.

## Decision

Den will replace flat per-turn step ceilings with a typed **turn budget policy** and **turn budget state** whose primary job is to track spend and loop health, not simply count continuations.

### 1. Turn budgets are profile-owned typed policy and budget ledger

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

### 2. Hard step ceilings remain only as emergency fuse

Den keeps a hard continuation ceiling because runaway loops are still a real infrastructure and UX risk.

But step count is not the primary budget dimension anymore. `emergency_hard_steps` is a deadman switch, not the main policy surface.

### 3. Primary budgets are wall-clock and tool-class spend

The first-class budget dimensions are:

- wall-clock time for the turn;
- total tool-call count;
- per-class tool-call quotas such as read/search/fetch/execute/write/destructive;
- consecutive tool failure streaks;
- ko-style repeated same-position retries.

This lets Den distinguish productive long work from expensive or risky churn.

### 4. Permission approvals are not themselves budget spend

Permission waits and approvals are workflow friction, not proof of runaway behavior.

Den must budget what happens after approval, not merely that an approval occurred. A run must not fail simply because it encountered many legitimate permission handshakes.

### 5. Rule of ko blocks repeated same-position retries

Den adopts a **rule-of-ko** style guard for agent loops:

- repeating the same tool signature too many times in a row is illegal;
- “same signature” means same tool plus normalized arguments;
- once the ko limit is hit, the loop stops even if hard steps remain.

This is the primary churn guard for long-running stances such as `work`.

### 6. Repeated failures have their own budget

Den separately tracks consecutive failed tool batches.

If the model keeps driving failure without recovering, the loop stops even if the hard step budget is not exhausted. This distinguishes useful long investigation from blind retry behavior.

### 7. Work gets a larger total budget than interactive pair/chat

`work` is expected to support materially longer runs than `pair`, `chat`, or `watch`.

The runtime therefore must support materially larger wall-clock budgets, total tool-call budgets, and class-specific budgets for `work`, while still applying the same ko/failure protections.

### 8. The decision is core runtime policy, not edge behavior

As with approvals and client obligations in ADR-0048, continuation-budget decisions are core runtime semantics.

BearWire and ACP may project the resulting stop reason, but they do not decide whether the model is allowed to continue.

## Consequences

### Positive

- Productive multi-tool turns are budgeted by actual spend and loop health instead of by a small flat continuation count.
- `work` can support longer runs without disabling loop safety.
- Repeated same-call churn is blocked explicitly instead of being indirectly caught only by a coarse step limit.
- Failure loops are treated separately from exploratory progress.
- The policy remains typed and role-owned.

### Negative / tradeoffs

- The loop controller now carries more state.
- Tool-class quotas are policy choices that will need tuning with real usage.
- Some borderline cases will still stop early or late until the policy evolves with more signals.

## Initial policy shape

The first implementation should use:

- a profile-owned wall-clock budget;
- profile-owned total and per-class tool-call budgets;
- consecutive tool failure cutoff;
- ko-style repeated identical tool signature cutoff.
- a high emergency hard-step fuse.

Token-aware continuation budgets remain compatible future extensions, but are not required for the first implementation because Den already tracks context-window budget separately in ADR-0047.

## Implementation notes

- `pair` and `chat` should keep moderate wall-clock and tool-call budgets.
- `work` should receive significantly higher wall-clock and tool-call budgets than interactive stances.
- The emergency step fuse should be high enough that it only catches pathological loops or missing health signals.
- Model-visible low-budget warnings are desirable, but runtime enforcement still remains authoritative.

## Non-goals

- No unbounded “just keep trying” loop mode.
- No ACP-only or BearWire-only loop heuristics.
- No free-form transcript string matching to detect loops.
- No planner-only execution gate as a substitute for core continuation policy.
