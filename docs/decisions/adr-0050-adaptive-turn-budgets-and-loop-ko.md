# ADR-0050: Adaptive Turn Budgets and Rule-of-Ko Loop Detection

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

The Den-native loop currently uses a small, mostly flat `max_steps` ceiling as its primary protection against runaway tool loops. That protected infrastructure and user experience, but in practice it also strangled useful multi-step work.

The failure mode is especially visible when a model is productively exploring a codebase or recovering from a failed tool call: a single fixed step ceiling treats productive search, partial recovery, and obvious churn as the same thing.

This becomes more limiting as `work` grows into a long-running stance. We want Den to allow materially longer runs where appropriate, while still stopping models that are thrashing, repeating the same tool calls, or blindly re-driving failed actions.

## Decision

Den will replace flat per-turn step ceilings with a typed **turn budget policy** and **turn budget state**.

### 1. Turn budgets are profile-owned typed policy

Each role profile owns a typed `TurnBudgetPolicy` with at least:

- `soft_steps`
- `hard_steps`
- `max_consecutive_tool_failures`
- `max_same_tool_signature_repeats`

This policy is part of the loop capability profile, not ad hoc string matching in BearWire or ACP.

### 2. Hard ceilings remain

Den keeps a hard ceiling because runaway loops are still a real infrastructure and UX risk.

But the hard ceiling is no longer the only decision rule. A turn can continue beyond the soft budget when it is still making progress.

### 3. Soft budgets require progress to extend

After `soft_steps` is reached, a continuation is allowed only if the most recent tool batch shows progress.

For the initial implementation, “progress” is intentionally simple and typed:

- a different tool signature than the immediately prior continuation batch; or
- a batch that materially changes the tool pattern instead of replaying the same call.

This avoids requiring speculative semantic scoring in the loop controller.

### 4. Rule of ko blocks repeated same-position retries

Den adopts a **rule-of-ko** style guard for agent loops:

- repeating the same tool signature too many times in a row is illegal;
- “same signature” means same tool plus normalized arguments;
- once the ko limit is hit, the loop stops even if hard steps remain.

This is the primary churn guard for long-running stances such as `work`.

### 5. Repeated failures have their own budget

Den separately tracks consecutive failed tool batches.

If the model keeps driving failure without recovering, the loop stops even if the hard step budget is not exhausted. This distinguishes useful long investigation from blind retry behavior.

### 6. Work gets a larger total budget than interactive pair/chat

`work` is expected to support materially longer runs than `pair`, `chat`, or `watch`.

The runtime therefore must support larger `soft_steps` and `hard_steps` for `work`, while still applying the same ko/failure protections.

### 7. The decision is core runtime policy, not edge behavior

As with approvals and client obligations in ADR-0048, continuation-budget decisions are core runtime semantics.

BearWire and ACP may project the resulting stop reason, but they do not decide whether the model is allowed to continue.

## Consequences

### Positive

- Productive multi-tool turns can continue beyond a small flat ceiling.
- `work` can support longer runs without disabling loop safety.
- Repeated same-call churn is blocked explicitly instead of being indirectly caught only by a coarse max-step limit.
- Failure loops are treated separately from exploratory progress.
- The policy remains typed and role-owned.

### Negative / tradeoffs

- The loop controller now carries more state.
- “Progress” is heuristic and intentionally local; it does not prove semantic progress.
- Some borderline cases will still stop early or late until the policy evolves with more signals.

## Initial policy shape

The first implementation should use:

- a profile-owned soft budget;
- a profile-owned hard budget;
- consecutive tool failure cutoff;
- ko-style repeated identical tool signature cutoff.

Wall-clock and token-aware continuation budgets remain compatible future extensions, but are not required for the first implementation because Den already tracks context-window budget separately in ADR-0047.

## Non-goals

- No unbounded “just keep trying” loop mode.
- No ACP-only or BearWire-only loop heuristics.
- No free-form transcript string matching to detect loops.
- No planner-only execution gate as a substitute for core continuation policy.
