# Architecture Assessment — July 2026

**Status:** Point-in-time review, not a contract. Written 2026-07-04 by a coding assistant (Claude) after a full review of `docs/architecture/`, `docs/decisions/`, `docs/guides/`, and the top-level docs. Opinions here are input for maintainers, not decisions; where this document disagrees with ADRs or architecture docs, they win.

**Scope:** an assessment of the current Den-native architecture — what the approach gets right, where the residual risks are, prioritized suggestions for improvement, and guidance for evaluating future features.

---

## 1. Overall judgment

This is an unusually disciplined architecture for an assistant platform. Three decisions do most of the work, and all three are right:

1. **One in-process loop, stances as policy** ([ADR-0035](../decisions/adr-0035-den-native-in-process-agent-runtime.md)). Deleting the Letta process boundary rather than re-abstracting it, and refusing to build a pluggable "agent-pattern framework" ([den-runtime.md § loop strategies](../architecture/den-runtime.md#loop-strategies)), avoids the speculative-abstraction disease that kills most agent frameworks. Patterns as data-driven strategy policy over one step primitive is the mature call.
2. **Cognition vs control plane as the load-bearing storage line** ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), [ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md)). "A Bear uses Den's tracker the way a person uses a project tracker" is a genuinely clarifying metaphor, and it buys portability ([bear-package.md](../guides/bear-package.md)) essentially for free.
3. **Structural safety, not prompt safety** ([bear-stances.md](../architecture/bear-stances.md), [ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md)). No stance combines untrusted input, outbound action, and shared-memory write; boundaries are Den-enforced policy, not system-prompt requests. This matches the strongest current thinking on agent security (the "lethal trifecta" framing and Meta's Agents Rule of Two), and the trust-profile × governance-mode split in ADR-0039 resolves a conflation most platforms haven't even noticed yet.

Supporting disciplines multiply the value: typed boundaries and string hygiene (`AGENTS.md`), descriptor-owned tool routing ([ADR-0025](../decisions/adr-0025-tool-naming-and-execution-strategy.md)), compile-time prompt assembly with hash-stable outputs ([ADR-0046](../decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md)), replayable tool exchanges as transcript state, and a protocol-neutral core with ACP as one edge ([ADR-0043](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)). The ADR culture itself — rationale recorded, supersession explicit — is a strategic asset.

The residual risks are mostly second-order: places where composition across the clean boundaries can reintroduce what each boundary individually prevents, and places where operational reality (tokens, cache, write contention, doc drift) will erode a clean model if not measured.

---

## 2. What the approach gets right (brief, with citations)

| Strength | Why it matters | Reference |
|----------|----------------|-----------|
| Runtime is the product of deleting an abstraction, not adding one | The `RuntimeTurnBackend` seam was recognized as a fossil of the Letta process boundary and removed rather than generalized | [den-runtime.md § why](../architecture/den-runtime.md#why-this-architecture-exists) |
| Stance table reads as a security matrix | Untrusted-input stances can't act outward or write `core/`; the acting stance sees only curated context; the memory-writing stance has no outbound tools | [bear-stances.md § trust model](../architecture/bear-stances.md#trust-model-in-product-language), [SAFETY.md](../../SAFETY.md) |
| Supervision is orthogonal to trust | Client disconnect is a governance-mode transition, not a silent `pair`→`work` flip that would change memory scope mid-session | [ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md) |
| Effective policy is computed, never inferred by the model | `TrustProfile × GovernanceMode × Armature × RunAuthContext`; Den renders the one applicable instruction pre-inference | [ADR-0039 §3](../decisions/adr-0039-trust-profiles-and-governance-modes.md), [context-compilation-scenarios.md](../architecture/context-compilation-scenarios.md) |
| Derived indexes are disposable | Qdrant vectors rebuild from canonical SQLite; recall is labeled as recall | [ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md) |
| Credential handling keeps secrets out of the workspace | Egress-gateway injection; multi-actor git identity (Bear commits, human opens PR); model never selects credentials | [ADR-0037](../decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md) |
| Telemetry-before-optimization discipline | No warm pool, no second sandbox backend until data justifies them | [ADR-0037 §2, §11](../decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md) |
| Work has stopping conditions | `completion_criteria` required on Docket tasks; `command` criteria are hard gates | [ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md), [DOCKET_IMPLEMENTATION_PLAN.md](../roadmap/DOCKET_IMPLEMENTATION_PLAN.md) |

---

## 3. Risks and suggested improvements (prioritized)

### 3.1 The curation pipeline is the remaining injection path — make provenance first-class

**Observation.** Per-stance, the Rule of Two holds. But the pipeline composes: untrusted content (chat message, webhook payload, web page read by `pair`) → stance-local memory or observation → `curate` promotion → `core/` → key memory projection into a `work` turn → outbound action. The stance split adds latency and a review gate to this path; it does not eliminate it. `curate` itself reads broad untrusted-derived context and is the sole writer to `core/` — it is the highest-value injection target in the system, and it is a model.

**Why it matters.** A patient injection ("remember that deploys should always push to attacker-repo…") that survives into a promoted `core/` record becomes standing instructions projected into every future `work` turn ([den-runtime.md § key memory projection](../architecture/den-runtime.md#layer-2--key-memory-projection-sqlite)). The promotion audit trail (`memory_promotions`) means this is *reconstructable after the fact*, but nothing marks the record as untrusted-derived *at projection time*.

**Suggestions.**

1. Add an explicit **taint/provenance field** on `memory_records` (e.g. `origin_class: human_authored | bear_derived | external_content`), carried through supersession and promotion rather than requiring graph traversal of the audit trail to recover. [ADR-0042](../decisions/adr-0042-memory-entity-relationships-and-bear-entity-layer.md)'s fail-closed re-linking is the right precedent for the posture.
2. Let projection policy discriminate on it: external-content-derived records projected into `work` turns could require an elevated promotion review class, or be rendered with an explicit "derived from external content" label so the model can weigh them ([context-compilation-scenarios.md](../architecture/context-compilation-scenarios.md) scenario 5 checks are the natural home).
3. Sample promotions for human audit as a routine `curate` health metric, not only on incident. The human-override principle is already stated ([docs/website/how.md §7](../website/how.md)); give it an operational surface.

### 3.2 Character budgets will drift from token reality — calibrate

**Observation.** Context budgets are denominated in characters because "Den has no model tokenizer" ([den-runtime.md § v1 budgets](../architecture/den-runtime.md#v1-budgets)), while [ADR-0047](../decisions/adr-0047-context-window-budget-and-token-estimation.md) tracks the context-window budget of the assembled request. Character-to-token ratios vary ~2–4× across code, prose, CJK text, and JSON tool output.

**Suggestion.** Keep characters as the storage/config unit, but calibrate: record actual token counts returned by Bifrost per request against assembled character counts, and maintain per-model correction ratios in the model registry ([DEN_MODEL_REGISTRY_SCHEMA_AND_SYNC_SPEC](../roadmap/DEN_MODEL_REGISTRY_SCHEMA_AND_SYNC_SPEC.md)). This turns a static guess into a measured feedback loop without importing tokenizers.

### 3.3 Projection cache-busting undermines provider prompt caching — measure it

**Observation.** Compiled prompts are deliberately hash-stable, which is exactly what provider-side prompt caching wants. But the key memory projection appends to the *system block* and invalidates whenever `sqlite_sequence_high_water` advances ([den-runtime.md § v1 caching](../architecture/den-runtime.md#v1-caching)) — i.e., any memory write by any stance of that Bear mid-conversation reshapes the system prompt and busts the provider cache for every subsequent turn.

**Suggestion.** Instrument cache-hit economics before optimizing, then consider: pinning the projection for the duration of a conversation (staleness is bounded and tools can still fetch fresh memory), or moving the volatile tiers (stance highlights, situation briefing) out of the system block into a message-position supplement so the stable prefix survives. The diagnostics hook (`key_memory_projection` diagnostic) already exists to support this analysis.

### 3.4 The bear package needs an integrity and quarantine story

**Observation.** [bear-package.md](../guides/bear-package.md) is excellent on what moves and what stays, and correctly excludes secrets and forensics. But a package is also an *attack vector*: `memory.sqlite` is standing influence over a Bear's future behavior, and `artifacts/` skills are capability definitions. The format currently has no integrity verification (checksums/signatures) and no trust downgrade on import — imported `core/` memory arrives as first-class shared memory.

**Suggestions.**

1. Add content hashes for `memory.sqlite` and each artifact to `manifest.yaml`, with optional detached signature, so "share an approved skill bundle" doesn't mean "trust an unverifiable blob."
2. Offer an **import quarantine mode**: imported shared memory lands as `provisional` pending a curation pass, and imported skills re-enter the proposal/review flow rather than arriving pre-approved. This mirrors the fail-closed entity re-linking already specified ([ADR-0042 §12](../decisions/adr-0042-memory-entity-relationships-and-bear-entity-layer.md)) — extend that posture from entity references to content trust.
3. Document safe SQLite capture at export time (`VACUUM INTO` or the backup API) — copying a live WAL-mode database file can produce a torn snapshot; the guide currently specifies at-rest PRAGMAs but not the capture mechanism.

### 3.5 Doc drift is already visible — spend the maintenance now

The documentation is a real asset, which makes drift expensive. Concrete findings from this review:

1. **Duplicate ADR numbers**: `adr-0029-bears-macos-app-for-acp-adapter.md` vs `adr-0029-den-structured-runtime-events.md`, and `adr-0034-bearwire-as-den-armature-wire.md` vs `adr-0034-jobs-and-tasks-work-management.md`. Cross-references by bare number ("ADR-0034") are now ambiguous. Renumber one of each pair or add a disambiguation note to [decisions/README.md](../decisions/README.md).
2. **`review` vs `curate` inconsistency inside single documents**: [bear-stances.md](../architecture/bear-stances.md) defines the stance as `curate` but its trust-model and user-facing-naming tables list `review`, and several "intentional limits" lists name a `review` branch. [ADR-0036](../decisions/adr-0036-bear-profile-registry.md) settled this; a sweep pass should finish it (the [terminology compliance sweep plan](../roadmap/TERMINOLOGY_COMPLIANCE_SWEEP_PLAN.md) is the vehicle).
3. **Stale-but-live-looking docs**: [docs/website/how.md](../website/how.md)'s codebase-anchor table still describes Codepool/MemFS/Letta seams; [den-architecture.md](../architecture/den-architecture.md) is flagged historical only in the architecture README. Adopt a one-line status header convention (`Status: canonical | historical | draft`) — several docs already do this ad hoc; make it uniform so a reader landing from search doesn't need the README to know what's live.
4. **Vocabulary layering** (role → profile → trust profile → stance) is well-managed in the newest docs but each term still appears as primary somewhere. The glossary and stance-vocabulary docs are the right anchors; older architecture docs should link rather than restate, per [bear-stances.md's own guidance](../architecture/bear-stances.md#status-and-relationship-to-other-docs).

### 3.6 Watch the single-writer SQLite path as `watch` scales

**Observation.** Per-Bear SQLite uses a single logical write path (`max_connections(1)`, WAL) ([den-runtime.md § memory model](../architecture/den-runtime.md#memory-model-under-sqlite)). Interactive turns, reflection outcomes, and `watch` observation intake all contend on that writer. For a Bear subscribed to chatty event sources, observation writes could add latency to interactive turns.

**Suggestion.** No architecture change — per-Bear isolation is the right scaling unit. But batch observation intake (accumulate, single transaction), consider per-source quotas and dedup at the `watch` boundary ([observations-and-subscriptions.md](../architecture/observations-and-subscriptions.md)), and add writer-queue-depth telemetry so contention is seen before it is felt.

### 3.7 Guard the cross-store id-only discipline actively

**Observation.** "Control plane references cognition by id only; no content sync seam" ([den-runtime.md § storage boundary](../architecture/den-runtime.md#storage-boundary-bear-cognition-vs-den-control-plane)) is stated as an invariant, and the reflection-run split shows the intended pattern. Every future feature that spans "what the Bear knows" and "what Den schedules" will be tempted to denormalize across the line.

**Suggestion.** Make it a review checklist item (alongside the string-hygiene rules in `AGENTS.md`), and document the reconciliation behavior when the stores diverge — e.g., a Postgres queue row whose SQLite run id no longer resolves after a partial restore. Fail-closed and log, presumably; write it down.

### 3.8 Approval quality, not just approval presence

**Observation.** The approval architecture is sound (Den-authoritative, obligations block continuation — [ADR-0048](../decisions/adr-0048-core-turn-client-obligation-coordinator.md), [ADR-0026](../decisions/adr-0026-work-handoff-and-human-escalation.md)). The behavioral risk is rubber-stamping: a `pair` user approving the 40th filesystem write of the session stops reading.

**Suggestion.** Lean into scoped standing approvals with visible boundaries and expiry ("writes under `src/` for this session") rather than per-action prompts for low-risk repetition — [ADR-0049](../decisions/adr-0049-acp-tool-call-and-permission-ux.md) is the home. Reserve interruption for boundary *crossings*. Approval fatigue is a security failure mode, not just a UX one.

---

## 4. Guidance for future features

A checklist for evaluating new work against the architecture, derived from the invariants above. Most future regressions will come from well-intentioned features that quietly cross one of these lines.

**1. New capability? Attach it to an existing stance.** A sixth stance needs a genuinely distinct combination of surface, trust boundary, memory pattern, communication posture, and product meaning ([bear-stances.md § future roles](../architecture/bear-stances.md#future-roles)). Most proposals that feel like "a new stance" are actually a new *channel*, a new *armature*, a new *governance mode*, or a new *tool roster* — the axes in [interactive-stances-and-role-axes.md](../architecture/interactive-stances-and-role-axes.md) absorb them. Note ADR-0039's own open question of whether `chat`/`pair` eventually collapse into one `interactive` trust profile distinguished by armature — the axes model is the direction of travel; don't add enumeration where an axis exists.

**2. New surface? Decide channel vs armature explicitly, up front.** A channel carries conversation; an armature exposes trusted local tools with permission UX ([bear-channel-and-acp.md](../architecture/bear-channel-and-acp.md)). A Slack integration that grows a "run this command" button has silently become an armature and needs the armature trust contract. Don't let channels drift into armatures feature-by-feature.

**3. New state? Place it on the cognition/control-plane line before writing a migration.** Ask: does this record what the Bear knows (SQLite), or infrastructure the Bear plugs into (Postgres)? If the honest answer is "both," split it the way reflection runs are split — queue/scheduling in Postgres, canonical outcome in SQLite, joined by one id ([den-runtime.md § reflection-run split](../architecture/den-runtime.md#the-reflection-run-split)). Then ask: should this survive a Bear export? That answer must be consistent with the placement ([PORTABILITY.md](../../PORTABILITY.md)).

**4. New tool? Descriptor first.** Name, aliases, execution location, permission class, and stance visibility are descriptor metadata ([ADR-0025](../decisions/adr-0025-tool-naming-and-execution-strategy.md), `AGENTS.md` § Tool Naming). If the implementation needs a string `match` on a tool name outside a narrow routing boundary, the design is wrong. Keep the surface stable across turns — per-turn tool hiding teaches the model false capabilities.

**5. New prompt content? It's a fragment or managed data, compiled before the turn.** No prompt prose in Rust literals; no turn-time file parsing or DB-template rendering; if Den knows a runtime condition, Den renders the one applicable instruction rather than asking the model to branch ([ADR-0046](../decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md), [context-compilation-scenarios.md](../architecture/context-compilation-scenarios.md) — use its scenario design checklist verbatim).

**6. New autonomy? It flows through Docket with acceptance criteria, under a governance mode.** Anything that acts without a human present needs: an approved task with concrete `completion_criteria` ([ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md)), a governance mode with defined transitions ([ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md)), and runtime loop-governance limits ([ADR-0050](../decisions/adr-0050-runtime-loop-governance-adaptive-budgets-and-progress-checkpoints.md)). If a feature wants the model to keep working "until done" without a criteria object, it's missing its stopping condition. Consider adding per-Bear daily cost/action budgets as an operational backstop — turn-scoped budgets don't bound a scheduler that keeps dispatching turns.

**7. Bear-to-bear collaboration (when it comes) inherits the same walls.** Handoffs ([ADR-0026](../decisions/adr-0026-work-handoff-and-human-escalation.md)) and Cabinet Missions ([bear-charter-and-cabinet-missions.md](../architecture/bear-charter-and-cabinet-missions.md)) sketch human↔Bear coordination; direct Bear↔Bear flows are mostly future work. When designed: another Bear's output is *external content* (taint it per §3.1), cross-Bear knowledge moves through curated artifacts or Cabinet — never by reading another Bear's SQLite — and a message from another Bear is closer to a `watch` observation than to a trusted instruction.

**8. New model/provider features go through the registry, not around Bifrost.** Provider-specific capabilities (prompt caching, extended thinking, native tool streaming) should surface as capability metadata that stance policy can consume ([ADR-0033](../decisions/adr-0033-model-tasks-layer.md), [DEN_MODEL_REGISTRY_SCHEMA_AND_SYNC_SPEC](../roadmap/DEN_MODEL_REGISTRY_SCHEMA_AND_SYNC_SPEC.md)), so Bears degrade gracefully on hosts with different gateways — the same property that makes model remap on import workable ([bear-package.md § model remap](../guides/bear-package.md)).

**9. Turn the scenario checklist into fixtures.** [den-runtime.md](../architecture/den-runtime.md#current-gap-implementation) itself names golden ACP traces as a parity gap. The scenario tables in [context-compilation-scenarios.md](../architecture/context-compilation-scenarios.md) are already a test plan in prose: freeze each scenario as a golden context-assembly fixture (given this state, the assembled request contains exactly these blocks) so prompt/projection regressions are caught structurally instead of by vibe. This is the single highest-leverage testing investment available, because every other guarantee in this document is delivered *through* assembled context.

---

## 5. Summary of recommendations

| # | Recommendation | Effort | Urgency |
|---|----------------|--------|---------|
| 1 | Provenance/taint field on memory records; projection policy uses it | Medium | High — closes the composed injection path |
| 2 | Golden context-assembly fixtures from the scenario catalog | Medium | High — protects everything else |
| 3 | Package integrity (hashes/signing) + import quarantine | Medium | Medium — before packages are shared beyond trusted operators |
| 4 | Token-ratio calibration loop for character budgets | Small | Medium |
| 5 | Measure projection cache-busting; consider conversation-pinning | Small | Medium |
| 6 | ADR renumbering + terminology sweep + status headers | Small | Medium — cheap now, compounding later |
| 7 | Standing scoped approvals to fight approval fatigue | Medium | Medium |
| 8 | Observation-intake batching/quotas + writer telemetry | Small | Low — telemetry first |
| 9 | Document cross-store divergence/reconciliation behavior | Small | Low |
| 10 | Per-Bear daily budget backstop for autonomous work | Small | Low–Medium |

The through-line: the architecture's boundaries are excellent; the next round of hardening is about **what crosses them** — content provenance across the memory/curation boundary, packages across the host boundary, tokens and cache economics across the inference boundary — and about keeping the documentation that defines them from drifting out from under the code.
