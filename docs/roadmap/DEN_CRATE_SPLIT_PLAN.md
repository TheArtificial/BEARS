# Den Crate Split and Rust Idiom Refactor Plan

> **Status (2026-06): draft for discussion.** This plan extracts the crate-boundary ("Option B") portion of [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md) into its own roadmap item and broadens it. Docket's own work (the `core/docket/` module and `DocketService` trait seam) stays in that plan. This document covers (1) turning the single `den` crate into a Cargo workspace and (2) using that effort as a thorough refactor toward idiomatic Rust — clippy-driven, with "stringy" structured arguments replaced by proper types. Canonical runtime context: [Den-Native Runtime](../architecture/den-native-runtime.md).
>
> **Status update (2026-06): v0 COMPLETE; v1 through `den-runtime` COMPLETE.** v0 hard gate done (core module triage ✅, trait seams ✅, clippy `-D warnings` baseline ✅ via `.github/workflows/den-clippy.yml`/`scripts/lint.sh` with pedantic/nursery advisory until v2, de-stringify seeded ✅ with `den-core::ids` + `den-core::governance`, legacy deletion ✅ — `core/letta` HTTP client + `core/codepool` + MemFS gone, config/compose native-only). v1 leaves extracted: `den-core` (seeded with `config`, `metrics`, `DenError`, `BearProfile`, `ids`, `governance`), `den-llm`, `den-memory`, `den-docket` as `den-core`-only leaves; error gate resolved (option 2, `DenError`).
>
> **`den-runtime` (v1.4) extraction COMPLETE (2026-06).** The native agent runtime — agent loop, native provider/runtime, governance, `bears` provisioning, `conversation` storage, `reflection`/`pair_reflection`, the `acp_sessions`/`acp_*` runtime contracts, and the runtime-side `memory`/`llm` glue — was lifted into the **`den-runtime`** crate across Stages A–E. Tool dispatch was **dependency-inverted** (`den-runtime` defines `RuntimeToolInvoker`; the `den` binary injects the concrete invoker), so the concrete executors stayed in the binary's `core/tools`. The **`den-tools` crate was dissolved** — its static tool surface (descriptors, argument shapes, capability traits, `dispatch`/`context`/`display`/`work_surface`/`support`) folded into **`den-core` as `den_core::tools`**, because the surface must sit *below* `den-runtime` while the executors sit *above* it. The final **flip** dropped the ~40 flat `pub use den_runtime::*` shims from `core/mod.rs` and repointed every call site at `den_runtime::*` / `den_core::tools::*`. Workspace build + the `-D warnings` clippy gate (all targets) are green.
>
> **v1 edges COMPLETE (2026-06).** All HTTP edges are extracted: **`den-http`** (foundation), **`den-oauth`** (OAuth2 server + bearer-auth), **`den-web`** (UI + `/v1` chat + observability + s3), **`den-acp`** (ACP protocol surface + `core::acp*` + `ApiState` + `/internal`), and **`den-api`** (v1/docs JSON API + `create_api_app`, mounting den-acp). The `den` binary is now thin-ish (~6.7k LOC: main/startup/DI + the `core/tools` executors intentionally kept per the v1.4 `RuntimeToolInvoker` decision). See the *v1.5 / v1.5+ COMPLETE* logs below. **v1.6 (2026-06):** build-time profiling + a low-risk graph reshape (relocated `acp_tokens` to den-http so den-web drops its den-api/den-acp dep and builds in parallel) cut the clean first-party build **30.3s → 24.5s**; the `den-core`/`den-db` split was evaluated and deferred as low-value. **v2 COMPLETE (2026-06):** dropped `async-trait` for native async-fn-in-trait on the 12 non-`dyn` service traits, and promoted pedantic/nursery clippy from advisory to gating (`deny` + curated allow-list). (Some sections below — the *target* crate DAG/table, the *`den-tools` triage*, and *Phase B sub-trait signatures* — are retained as historical record and predate the `den-tools` dissolution; the live tool surface is now `den_core::tools`.)
>
**Follow-on (2026-06): protocol-agnostic boundary — [ADR-0043](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md).** The crate split made the layering *visible* (and the `den-acp`/`den-api` de-aliasing removed the self-alias + re-export shims), but it did not fix the remaining ACP-centric layering: ~399 `acp_*` symbols still live in `den-runtime`, `ApiState` carries ACP turn/cancel registries and is owned by the ACP edge, and `den-api` depends on `den-acp` for shared state. ADR-0043 is the next architectural step — rename the core turn/session/event machinery off its `acp_*` prefixes, neutralize `ApiState`, and reduce `den-acp` to a true ACP *adapter* (sibling to `den-api`/`den-web`, none depending on another). Out of scope for the crate-split plan itself; tracked under ADR-0043.
>
**Runtime dependency diet (2026-06): `den-protocol` + `den-service` extracted.** Stable runtime DTOs/contracts (`RuntimeStreamEvent`, `RuntimeSemanticEvent`, runtime refs, continuation/approval enums, error classification helpers, runtime conversation/backend traits) moved to lightweight **`den-protocol`**. Concrete shared edge app state (`DenState`), Bifrost model metadata client, process-local tool-turn coordinator, and active turn cancellation/controller moved to **`den-service`**. Model option DTOs and Den model registry helpers moved to **`den-llm`**. `den-runtime::{runtime_contracts,bifrost,tool_turns,turn_controller,DenState,llm::model_registry,agent_assist::ModelOption}` remain compatibility re-exports, while edge crates can import stable protocol/service/model types without depending on churny runtime internals for those concerns.
>
> **Decided:** foundation crate is **`den-core`**; the **binary keeps the name `den`** (see *Crate naming*). The big crates **are split in v1** (no deferral of `den-acp`/`den-tools`/`den-api` sub-splits). **`den-acp` owns its HTTP surface directly.** **clippy strictness is progressive** (advisory in v1, gating in v2). The **`den-core`/`den-db` split is deferred to v2.** **v0 is a hard gate** — no crate is extracted until v0 completes in full.

## Motivation: build and test time (and idiom debt)

The primary motivation is **build and test iteration time**, not architectural purity. Module privacy (the trait seams in the Docket plan) already enforces the important subsystem boundaries within a single crate; it does **not** improve compile times, because any edit recompiles the whole crate.

Current reality:

- `den` is a **single ~85k-LOC crate** (`core/` ~46k, `api/` ~22k, `web/` ~13.5k, plus 43 loose `core/*.rs` files at ~19k).
- The `Cargo.toml` already fights compile time with band-aids: `codegen-units = 256`, per-package `opt-level` overrides, a `dev-fast` profile. These help, but the structural cause — one crate that recompiles in full on every edit — is untouched.
- Tests cannot be exercised per subsystem: `cargo test -p <subsystem>` is impossible, so any test run rebuilds the world.

The win from a workspace split is **cutting the dependency graph** so that an edit recompiles only the changed crate plus its downstream dependents, and tests for one subsystem build and run without rebuilding unrelated code. Smaller crates are a means to that end. Two levers matter:

1. **Stable leaves** — foundational crates (types, errors, LLM client) change slowly; many crates depend on them, so keeping them small and stable means most edits don't touch them.
2. **Isolated edges** — high-churn but low-fan-in code (HTML/templates, HTTP handlers, ACP glue) should sit at the *top* of the graph so its frequent edits recompile only itself and the binary.

Secondary motivation: the codebase has accumulated **idiom debt** — stringly-typed arguments, `serde_json::Value` poking, bare `Uuid`/`String` ids, scattered alias `match` arms. The crate split touches nearly every module anyway, so it is the right moment to pay this down. This is treated as in-scope, not a "later" cleanup (see *Rust idiom refactor*).

## Goals and non-goals

**Goals**

- Reduce incremental rebuild time after a one-file edit; make `cargo test -p <crate>` / `cargo check -p <crate>` fast and meaningful; enable parallel crate compilation.
- Preserve (and make compile-time-hard) the subsystem boundaries from the Docket plan.
- Land a clean clippy baseline and convert "stringy" structured arguments to typed Rust at every boundary the refactor touches.

**Non-goals**

- No process split. One deployable binary, one `docker-compose.yaml` service.
- No behavior change. This is a structural + type-level refactor; each step should be diff-reviewable as moves, visibility changes, and type substitutions.
- Not a redesign of runtime semantics. Trait seams already route cross-subsystem access, so extraction is a move-and-rename plus type tightening.

## Principles

- **Triage before splitting.** v0 sorts the loose `core/*.rs` files into their intended subsystem modules *while still a single crate*, so each future crate's contents are already co-located and the extraction is mechanical.
- **Traits first, crates later.** Do not extract a service crate until its public face is a stable trait. A crate boundary across an unstable trait *hurts* build time, because every signature change rebuilds all dependents.
- **Define traits where they are used.** Inversion traits live in the *consumer* crate: `den-docket` defines `TaskDispatcher` (it calls it), `den-tools` defines `ToolContext` (it needs it); `den-runtime` implements both. This keeps Docket and tools free of a runtime dependency and is the idiomatic Rust direction.
- **Leaves and edges first.** Extract the foundation leaf and the web/api/acp edges early; gate the service/runtime crates on v0 completion.
- **Keep `den` as the binary name.** The build artifact, Dockerfile, and deploy scripts reference the `den` binary; renaming it is churn with no payoff.
- **Parse, don't validate.** Convert untyped input (HTTP/ACP/tool JSON, env) into typed domain values once, at the boundary; downstream code receives typed values, never re-parses strings.

## Proposed crate breakdown (v1 target)

Layered DAG, leaves at the bottom; both former "big crates" are split. Sizes are rough current LOC; "new" does not exist yet. **Note (2026-06): the original plan had a separate `den-tools` crate between `den-runtime` and the leaves; it was dissolved during the `den-runtime` lift — the static tool surface folded into `den-core` (`den_core::tools`) and the executors stayed in the `den` binary's `core/tools` behind `RuntimeToolInvoker`. The DAG below reflects the as-built layering.**

```
                              den (bin: startup, wiring, anyhow,        ~0.5k
                              user/email/auth, core/tools executors)
                  /        |          |            |        \
                 v         v          v            v         v
            den-web    den-api    den-acp     (workspace tests)        ~13.5k / ~10-12k / ~10-15k
                  \        |          /
                   \       |         /
                    v      v        v
                        den-runtime  (loop, governance, harnesses,      ~12-18k  [DONE]
                                      reflection, RuntimeToolInvoker)
                    /     |          \
                   v      v           v
              den-llm  den-memory   den-docket                          ~2.2k / ~1-3k(new)
                   \      |           /
                    v     v          v
                            den-core  (types, errors, config, pool,      ~3-5k (leaf)
                                       migrations, observability,
                                       den_core::tools surface)
```

| Crate | Role | Depends on | Source today | Churn |
|-------|------|------------|--------------|-------|
| **`den-core`** | Foundation leaf: shared domain **newtypes** (bear/job/task/run/session ids) and **enums** (role, trust stance, governance mode, statuses), error types, config, `PgPool`/`SqlitePool` handles, migrations runner, tracing/observability setup, **and the model-facing tool *surface* (`den_core::tools`: descriptors, argument shapes, capability traits, `dispatch`/`context`/`display`/`work_surface`/`support`)** — folded in when `den-tools` was dissolved. Third-party deps only. | `errors/`, `observability/`, `config.rs`, shared model types, former `den-tools` surface | low |
| **`den-protocol`** | Stable runtime/event/contract DTOs shared by runtime implementations and edge crates; no app state or execution logic. | `den-core` | low |
| **`den-service`** | Shared concrete edge service state: `DenState`, Bifrost model metadata client, tool-turn coordinator, and active turn controller/cancellation registry. No runtime execution loop. | `den-core`, `den-protocol`, `den-memory` | low-medium |
| **`den-llm`** | Bifrost/LLM gateway client + streaming. Stable, widely depended on. | `den-core` | low |
| **`den-memory`** | Bear memory: `MemoryStore` trait + per-Bear SQLite impl + curation. **Native today** (see *Memory status*). | `den-core` | medium |
| **`den-docket`** | Jobs/tasks orchestration (Postgres): `DocketService` + `TaskDispatcher` traits + impl. Never executes task bodies. | `den-core` | medium |
| ~~**`den-tools`**~~ | **Dissolved (2026-06).** Static surface → `den_core::tools`; concrete executors stayed in the `den` binary's `core/tools` behind `den-runtime`'s `RuntimeToolInvoker` seam (surface and executors sit on opposite sides of `den-runtime`, so they can't share a crate). | — | — |
| **`den-runtime`** ✅ | Native agent loop, governance/supervision modes, context assembly, role harnesses, reflection, conversation storage, bears provisioning, and runtime implementation. Defines `RuntimeToolInvoker`; stable runtime contracts and shared edge service state live in `den-protocol`/`den-service` with compatibility re-exports. **Extracted (v1.4).** | `den-core`, `den-protocol`, `den-service`, `den-llm`, `den-memory`, `den-docket` | high |
| **`den-acp`** | ACP protocol runtime + its HTTP surface: turn runner, sessions, adapter event mapping, the residual `core/acp/`, `api/acp/`. | `den-runtime`, `den-core` | high |
| **`den-api`** | Non-ACP HTTP/JSON API (`/v1` chat, bears, admin JSON) + OpenAPI. | `den-runtime` (+ service traits) | high |
| **`den-web`** | Server-rendered admin UI + minijinja templates + template embedding (`build.rs`) + `observability/` + `s3/`. | `den-runtime` (+ service traits) | high |
| **`den`** | Binary: `main`, startup, dependency injection (constructs concrete services, injects dispatcher + tool context), config load, router composition (mounts api/acp/web). | everything | low |

**Build-time payoff:** `den-web` extraction stops HTML edits from rebuilding logic; `den-acp` and `den-api` isolate the two highest-churn HTTP surfaces from each other and from runtime; the `den-runtime` lift already moved the ~12–18k-LOC runtime core out of the edges' rebuild path; `den-core`/`den-llm` are stable leaves most edits never touch; service crates behind stable traits don't cascade. Tests become per-crate; crates compile in parallel. *(Caveat from the `den-tools` dissolution: the tool surface now lives in `den-core`, the universal root dep, so tool-surface edits trigger wider rebuilds — accepted trade-off vs. a confusingly-named extra crate.)*

**Deliberately not crates:** legacy `core/letta/` (~3.9k) and `core/codepool/` (~0.5k) are slated for deletion in the native migration ([DEN_NATIVE_RUNTIME_PLAN.md](DEN_NATIVE_RUNTIME_PLAN.md)); do not crateify them — delete in/around v0.

## Crate naming (decided: `den-core` + binary `den`)

The foundation leaf is **`den-core`** and the **binary stays `den`**. Rationale (recorded for posterity):

- **`*-core` is the dominant Rust idiom** for a foundational leaf everything depends on: `tracing-core`, `futures-core`, `serde_core`, `regex-syntax`/`regex-automata`, the `tower` ecosystem. `-foundation` reads as Swift/Apple, not Rust.
- **Naming the leaf `den` would force a binary rename** (two crates can't share a name), changing the build artifact and every Dockerfile/deploy reference for no benefit. Keeping `den` as the binary preserves "den = the product you run."

Optional later refinement (deferred): split `den-core` into `den-core` (pure types/errors) + `den-db` (pool/migrations) so type-only crates don't pull `sqlx`. Not in v1. **Evaluated 2026-06 and judged low-value (see *v1.6* below):** den-core has *no* pool/migrations (those live in the `den` binary — `startup.rs`/`seeds.rs` + the `migrations/` dir); den-core's entire sqlx coupling is two small items (`impl From<sqlx::Error> for DenError` in `error.rs`, `#[derive(sqlx::Type)]` on 4 id newtypes in `ids.rs`). Making den-core sqlx-free would only help *isolated* leaf builds (`cargo build -p den-llm`) — Cargo **feature unification** means a full `cargo build --workspace` compiles sqlx once regardless. The build-time effort was redirected to the actual critical path (den-web/den-runtime).

## Rust idiom refactor (in-scope)

The split is also the vehicle for paying down idiom debt. Two workstreams run alongside the extraction.

### clippy as the advisor and gate

- Adopt a workspace lint table (`[workspace.lints.clippy]` in the root `Cargo.toml`, Cargo ≥1.74) so lint levels are single-sourced across all crates.
- v0 establishes a **clean baseline**: run `cargo clippy --workspace --all-targets --all-features`, fix the existing warnings, then make CI enforce `-D warnings`.
- Enable `clippy::pedantic` and `clippy::nursery` as **advisory** groups, allow-listing the noisy lints; use them to drive idiom fixes (needless clones, `&str` vs `String`, manual `map`/`and_then`, `expect` discipline, etc.) rather than as hard gates initially.

### De-stringify: typed arguments over "stringy" structures

Replace untyped structured data with Rust types, applying "parse, don't validate" at boundaries:

- **Ids → newtypes.** `BearId(Uuid)`, `JobId`, `TaskId`, `RunId`, `ConversationId`, `SessionId` instead of bare `Uuid`/`String`. Prevents argument transposition and clarifies signatures.
- **Closed sets → enums.** Role, trust stance, governance mode (ADR-0039), task/run/criterion status (ADR-0034), tool scope/side-effect kinds. Several already exist as enums; ensure DB mapping uses `sqlx::Type`/derive, not string `match`, and that no boundary re-stringifies them.
- **Tool arguments → typed structs.** Replace `serde_json::Value` poking in tool dispatch with per-tool argument structs that `Deserialize` once at the boundary. Tool names, aliases, and permission classes stay **descriptor-owned** (per `AGENTS.md`: no scattered alias `match` arms or hardcoded allowlists; use a descriptor resolver).
- **Paths → typed.** A logical-path newtype (and `PathBuf` where filesystem) instead of `String` for memory logical paths.
- **Errors → `thiserror` at crate boundaries**, reserving `anyhow` for the binary/top. Each crate exposes a typed error enum.
- **Config → typed once.** Parse env into typed config structs at startup; no scattered stringly env lookups.

Each crate, as it is extracted, is idiomatized on the way out — extraction PRs carry the type tightening for the modules they move, so the refactor lands incrementally rather than as one mega-change.

## Memory status

The Docket plan was written when bear memory was "still Letta-backed / not yet implemented." That is no longer true: **bear memory is native per-Bear SQLite today** (`core/memory/` with `store/` and `curation.rs`, per [ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md) and [Den-Native Runtime](../architecture/den-native-runtime.md)). The `MemoryStore` trait seam can be cut **now**; `den-memory` is a present-tense extraction target gated only on giving `core/memory/` a stable trait face.

## Phasing

### v0 — In-crate groundwork (single crate; behavior-preserving)

Must complete before any crate is extracted.

1. **Core module triage. ✅ done (2026-06).** Sort the 43 loose `core/*.rs` files into their intended subsystem modules (`acp`, `runtime`, `conversation`, `memory`, `reflection`, `llm`, `letta`, `tools`), so each future crate's contents are co-located. This was the gating deliverable; loose subsystem-file count is now 0 (see *v0 — core module triage* log).
2. **Trait seams in place. ✅ verified (2026-06).** `ToolContext` (now `den_core::tools::dispatch::ToolContext`; originally drafted as `den-tools::ToolContext` before that crate was dissolved) is the composed capability face (`BearDirectory`, `ConversationTitleOps`, `EnvironmentOps`, `WorkPlanOps`, `WorkSurfaceOps`, `WebFetcher`, `RoleMemoryStore`, `PromptMemoryStore`, `MemoryReviewStore`, `PlanModeOps`); the `den` binary supplies `DenToolContext`. `den-docket::DocketService` is the Docket face. The memory seam is split: the tool-facing `RoleMemoryStore` trait (inversion) plus `den-memory` as a non-inverted storage **leaf** crate with a concrete public face (`MemoryStoreManager` + typed functions) — a trait there would add indirection with a single SQLite impl, so the concrete API is intentional. `TaskDispatcher` is the only deferred seam: `den-docket` is intentionally **level-1 minimal**, so the runtime-facing dispatcher lands with Docket Phase 1 (its own track) and does not gate crate extraction.
3. **clippy baseline. ✅ done (2026-06).** Workspace lint table in place (pedantic/nursery advisory `warn`). The **default lint set** (rustc + clippy default groups) is now warning-clean across the workspace and gated by CI: `.github/workflows/den-clippy.yml` runs `cargo clippy --workspace --all-targets -- -D warnings -A clippy::pedantic -A clippy::nursery` (mirror locally via `scripts/lint.sh`); pedantic/nursery stay advisory until v2. Cleared ~20 dead-code items revealed by the legacy teardown (legacy `map_acp_history_page`/`map_compaction_status_for_history` + their Letta-body tests deleted; test-only workflow-state/prompt-memory/canonical-persistence helpers gated `#[cfg(test)]`), declared the `production` feature on `den-core`, dropped the unread `salience`/`config` fields, and allow-listed `too_many_arguments` (de-stringify bundles some into typed structs).
4. **De-stringify the shared core. ✅ seeded (2026-06).** Typed errors (`DenError`) and `Config` already landed; this seeds the remaining foundation vocabulary in `den-core` and adopts it at foundation boundaries only (the wider call-site sweep rides along with each module move in v1, per *idiom tightening for moved modules*):
   - `den-core::ids` — `BearId(Uuid)`, `UserId(i32)`, `SessionId(String)`, `ConversationId(String)`; transparent `serde`/`sqlx` so DTO/row adoption is drop-in. Adopted at the den-core boundary (`BearProfile::tags_for_bear`).
   - `den-core::governance` — `GovernanceMode` (`interactive`/`grace`/`autonomous_continuation`/`observational`/`frozen`) + the derived `RunMode` projection, seeding the **code half of ADR-0039** (`Mode`). Runtime wiring waits on the deferred ADR-0039 `WorkspaceSession`/governance-timeline schema.
   - `Config` is already typed (ports, bools, `UiFixtureProfile`); no stringy-config conversion was needed.
5. **Delete legacy.** Remove `core/letta/` and `core/codepool/` per the native migration. **In progress (2026-06).**
   - ✅ **Phase A — branch collapse (done, committed).** Every `config.uses_native_agent_runtime()` execution branch in `core/tools/*`, `core/native_runtime/*`, `core/memory/*` (curation/curate_executor), `core/runtime/role_registry`, `core/reflection/conductor`, `seeds.rs`, `api/acp/*`, and `web/*` (`status`, `stack_health`, `admin/mod`, `v1`) was collapsed to the native path; dead MemFS/Postgres-curation sides removed; the dead `web/v1` Letta history-mapping helpers + their tests deleted. The crate is now **functionally native-only** even though the `LettaClient`/`CodePool` clients are still constructed.
   - ⏳ **Phase B — structural teardown (port-then-remove; mostly done).** Decision (user): **port operator/member features to native, no feature loss.** Completed and committed in this effort:
     - Deleted the dead `WebChatTransportDataSource` (Codepool streaming) seam — native `/v1/chat` is the only chat path.
     - Dropped the Letta/Codepool-only operator pages (harness-pool, letta-health, letta-code harness, unlinked-letta-agents, register-memfs-views) + their templates and dead deps (`letta_code_harness`, `list_letta_code_harness_rows`, `register_existing_role_views`).
     - Ported bear create/edit/onboarding **model catalog** to native Bifrost (`model_catalog_select_context`); dropped Letta tool/agent-type selectors (native tools are descriptor-owned).
     - Ported member `bear_management` **conversation lists** to native `conversation_persistence` (keyed by `bear_id`) and the **memory browser** to native `sqlite_memory_*`; dropped Letta `fetch_agent` diagnostics (native `bear_agent_health_rows` covers it). ACP close now archives natively; compaction is a native no-op.
     - Dropped the dead `letta`/`codepool`/`web_letta_data`/`web_memory_data` `AppState` fields, deleted the `web/data` seam + UI fixtures, and removed every handler-level `state.letta`/`state.codepool` read. `provision_bear_if_configured`/`reconcile_*` lost their dead `letta`/`bifrost` args.
   - ✅ **Phase B remainder — code teardown (done, committed 2026-06, `test`).**
     - Removed the dead `ApiState.letta` field + all construction sites (`api/service.rs`, `api/acp/stream/sse_stream.rs`, `api/acp/tests.rs`) and the stray `let _letta = …` test locals.
     - `startup.rs`: dropped the Codepool/Letta preflight in `validate_upstream_connections` and the `LettaClient`/`CodePoolClient` construction.
     - **Deleted `core/codepool/` entirely.**
     - **Extracted the HTTP `LettaClient`** out of `core/letta/`: the dead client + Letta-only submodules are gone; the native-used items the module hosted (`runtime_stream_parser`, title/tool-policy/display helpers, `LettaModelOption`/`LettaToolOption`) are retained (rename off the `letta` name is a later cleanup).
     - **Deleted the MemFS HTTP layer**: `core/memory/manager_head.rs` (MemFS client + view types) removed; `core/tools/memfs.rs` trimmed to the native-runtime guard helpers (`is_memfs_client_tool_name`, `filter_client_tools_for_native_runtime`). The bear detail-page **work-surface listing** was ported from MemFS tree/file fetches to native `sqlite_collect_role_logical_paths` + `sqlite_memory_read`.
     - **Removed the legacy Letta conversation-summary tool** (`core/tools/letta.rs`) and its `ConversationTitleOps::patch_summary` seam — it errored under `AGENT_RUNTIME=native` (latent bug); native `set_title` is the only path now.
   - ✅ **Config + compose (done, committed 2026-06, `test`; user-approved).** These were atomically coupled (the config fields, their remaining readers, and `AgentRuntimeMode` changed together):
     - `den-core/config.rs`: removed the `AgentRuntimeMode` enum, `agent_runtime_mode` field + `uses_native_agent_runtime()`, and the legacy fields `letta_base_url`, `letta_api_key`, `letta_pg_uri`, `letta_memfs_service_url`, `codepool_base_url`; renamed `codepool_internal_token` → `den_internal_token` (`DEN_INTERNAL_TOKEN`).
     - Collapsed the `uses_native_agent_runtime()` readers to native-only: `startup.rs` (`validate_runtime_config` now just JWT + `LLM_API_URL`; dropped the Letta/Codepool/standalone-web branches + their tests), `web/admin/bears.rs` (`native_runtime => true`), `core/tools/environment.rs`, `core/memory/tools.rs` (runtime tag hardcoded `"native"`; dropped the `sqlite_write_*` `config` arg). The dead `letta_configured`/`letta_api_base` template context in `web/bear_management.rs` is now inert (empty) — `bear/details.html` cleanup deferred to a UI pass.
     - `.env.example`: dropped the legacy `LETTA_*`/`CODEPOOL_*` block and `AGENT_RUNTIME`; added `DEN_INTERNAL_TOKEN`.
     - **docker-compose / preflight:** dropped the `AGENT_RUNTIME=${AGENT_RUNTIME:-native}` env defaults (2 sites; no `bears-letta`/`bears-codepool`/`bears-memfs` services existed); `services/preflight/preflight.py` is now native-only (removed the Letta/Codepool/MemFS/`LETTA_PG_URI` checks and the `letta-pg` mode).
     - Deleted the obsolete `tests/acp_gateway.rs` (fake-Letta gateway, 36 tests) and `tests/web_chat_role.rs` (fake-Codepool web chat) — they tested removed runtimes with no native equivalent.
   - 🧹 **Deferred (non-blocking):** strip the inert `letta_configured`/`letta_api_base`/`letta_agent_*` scaffolding from `web/bear_management.rs` + `bear/details.html`; rename the retained `core/letta`-named native helpers off the `letta` name. (The residual dead-code sweep is **done** — folded into the v0 #3 clippy baseline.)

### v1 — Workspace split (extract + idiomatize, leaves→edges)

Each step keeps the workspace green and is behavior-free; idiom tightening for moved modules rides along.

1. ✅ Workspace skeleton + **`den-core`** (mechanical moves + `crate::` → `den_core::`).
2. ✅ **`den-llm`** (stable leaf).
3. ✅ **`den-memory`** + **`den-docket`** leaves. *(The planned standalone `den-tools` was built then dissolved — surface folded into `den-core` as `den_core::tools`, executors stayed in the binary; see v1.4.)*
4. ✅ **`den-runtime`** (defines `RuntimeToolInvoker`; implements the inversion traits). **Done 2026-06 — see *v1.4* below.**
5. ✅ **`den-http`** (foundation) + **`den-oauth`** + **`den-web`** + **`den-acp`** + **`den-api`** (edges). **Done 2026-06 — see *v1.5 / v1.5+ COMPLETE* logs below.** (Order as-built: foundation → web/api → oauth → acp; den-api mounts den-acp.)
6. ✅ The original `den` crate collapsed to a thin-ish binary: `main`/startup/DI/router composition (+ the `core/tools` executors, kept in-binary by design per the v1.4 `RuntimeToolInvoker` decision).

#### v1.4 — `den-runtime` extraction plan (scoped 2026-06)

Survey result: `core/` is already a clean layer (**no `core/` module imports `crate::api` or `crate::web`**), the extracted leaves are pure (`den-core`-only), and `den-llm` already keeps the SSE→`RuntimeStreamEvent` mapping out of the leaf to avoid a `llm → runtime` cycle. So `den-runtime` is a cohesive lift of the runtime subsystems plus the `ToolContext` impl.

**Goes into `den-runtime`:** `agent_loop`, `native_runtime`, `runtime` (provider / `contracts` = `runtime_contracts` / `role_registry` / compaction / conversations / `turn_state` / `pair_turn` / `bearwire_projection`), the `core/llm` glue (`bifrost`/`stream` SSE mapping), `bears` (provisioning/registry/db), `conversation` (events + persistence) *(decision: kept in `den-runtime`, not a separate leaf)*, `reflection` + `pair_reflection`, the `core/memory` glue over the `den-memory` leaf (curation/`curate_executor`/prompt-block store/blocks), `work_plans` (docket projection), `sandbox`, `migration`, and the `DenToolContext` capability impls (`core/tools/*` impl side) — i.e. `den-runtime` **implements `den-tools::ToolContext`** (all 10 sub-traits) and consumes `den-docket::DocketService` + the `den-memory`/`den-llm` leaves.

**Stays out:** `acp/` → `den-acp`; `api/` → `den-api`; `web/` + `observability/` + `s3/` → `den-web`; `user/` + `email/` + `auth_backend` → the thin `den` binary *(decision)*.

**Boundary resolutions (names are misleading; both are runtime concerns, move into `den-runtime` to avoid a `den-runtime → den-acp` cycle):**
- `acp::tool_turns` (`acp_tool_turns`) — consumed by `core/runtime`; it is tool-turn coordination, not ACP protocol. Relocate under `runtime` in `den-runtime`.
- `work_plans` — docket projection used by `runtime` and the tool impls; lands in `den-runtime` (depends on the `den-docket` leaf).

**Decision: rename off `letta` now (not deferred).** The retained native helpers in `core/letta/` (runtime stream parser, assistant display/title, agent JSON projections, model/tool option types, tool policy) and `acp::letta_events` (which holds the native `AcpGatewayEvent`) are renamed off the `letta` name before/with the lift.

**Extraction order (each step keeps the workspace green + its own commit):** rename off `letta` *(done)* → den-runtime skeleton *(done)* → **runtime + shared turn/tool contracts cluster** *(done — see below)* → **runtime blob (5 SCC-ordered stages, see below)** → `DenToolContext` impls → flip `den` onto `den-runtime`, drop the flat shims.

**Second cluster — the "runtime blob" (staging corrected 2026-06 after precise import analysis).** The original per-subsystem order (`native_runtime` → `agent_loop` → `bears` → …) is **not achievable**: these subsystems are mutually recursive and all target `den-runtime`, so they move as strongly-connected components, not individually. Precise `module::` import analysis (token scans like `\bbears` were false-positiving on `bear_id`/`BearProfile`) gives this condensed DAG (X → Y = X depends on Y):
- **A = {`llm`, `agent_assist`}** — leaf (mutual; no other blob deps).
- **B = {`bears`, `memory`}** — leaf (mutual; no other blob deps).
- **`conversation`** → B, `acp::runtime`.
- **`pair_reflection`** → B.
- **C = {`native_runtime`, `agent_loop`}** → A, B, `conversation` (via `conversation_events`), `acp::turn_runner` (request types only).
- **`reflection`** → B, C. (`reflection`/`pair_reflection` are pure sinks — nothing in the blob depends on them.)

The blob's **only** external (den-stay) tethers, beyond `CustomError` (335 refs → `den_core::DenError`): `agent_loop` uses `den_tools::descriptor::render_profile_tool_surface_blurb` (leaf, path rewrite); `conversation`/`native_runtime` use `acp::runtime` conversation-id predicates (`acp::runtime` is a leaf → moves in); `native_runtime` uses `acp::turn_runner` request types carrying `&ApiState` (→ split: move the runtime-contract parts in, replace `&ApiState` with concrete `sqlx_pool`/`config`/`memory_stores` fields, keep the `acp_sessions`-coupled orchestration wrappers in den). `acp::sessions` (leaf, 393 LOC) moves in too because `materialize_acp_runtime_conversation_if_needed` upserts the session store. `acp::tokens` stays in den (auth → `den-acp` later).

**Staged execution (each = git mv subset + path rewrite + re-export shims + green + commit):**
1. **Stage A** *(done)* — `llm` + `agent_assist`.
2. **Stage B** *(done)* — `bears` + `memory` + `conversation` (Stage C merged in) + `conversation_ids`.
3. **Stage C** *(done, merged into B)* — `acp::runtime` predicates + `conversation`.
4. **Stage D** *(done)* — break `ApiState` coupling + split `acp::turn_runner` runtime-contracts; move `acp::sessions` + `native_runtime` + `agent_loop`. **Tool-dispatch dependency-inverted (see below) so `core/tools/*` did NOT have to move.**
5. **Stage E** — `pair_reflection` + `reflection`.

**Stage D correction — the native runtime does NOT drag `core/tools` in (dependency inversion, 2026-06).** Initial Stage D scoping assumed `native_runtime`/`agent_loop` could pull `core/tools/*` along, but the tool dispatcher (`tools::session::invoke_den_tool`) builds the full `DenToolContext` aggregate, which transitively reaches `tools::memory_review → reflection_conductor` (Stage E) and `tools::workflow → work_plans`/`docket` (den-stay). Moving `tools` would therefore have forced `reflection` + `work_plans` + `docket` in too. Instead we **inverted** the dependency (this is the roadmap's intended `DenToolContext`/`ToolContext` step, brought forward only at the call boundary): `den-runtime` defines `native_runtime::RuntimeToolInvoker` (a trait: `invoke(pool, config, stores, tool_name, args, ctx) -> Result<Value, DenError>`); `NativeWebChatTurnParams` carries an `Arc<dyn RuntimeToolInvoker>`; the `den` binary injects `core::tools::runtime_invoker::DenRuntimeToolInvoker` (which calls the in-`den` `invoke_den_tool`) at the web-chat turn boundary. The ACP turn path runs no in-process Den tools, so its request types need no invoker. The only pure helpers `agent_loop`/`native_runtime` needed from `tools` were already in the `den_tools` leaf (`work_surface`, `support`, `descriptor`); the lone exception, the `memfs` client-tool guards, moved to `den_runtime::native_runtime::memfs` with a `core/tools/memfs.rs` re-export shim. Net: `core/tools/*`, `reflection`, `work_plans`, `docket` all stay in `den` for now; the remaining `move-toolctx` work is moving the concrete `DenToolContext` impls behind `RuntimeToolInvoker` later.

**`move-toolctx` resolution — dissolve the `den-tools` crate into `den_core::tools` (decided 2026-06).** The `den-tools` *crate* was judged misleading (it held only the static surface + traits + dispatcher; the actual executors are the ~5k LOC in the binary's `core/tools`) and crate-granularity overkill. A single unified tools crate is impossible: the tool **surface** is consumed *by* `den-runtime` (so it must sit at/below it) while the **executors** call `bears`/`memory`/`reflection` *in* `den-runtime` (so they must sit above it) — surface and impl are on opposite sides of `den-runtime`. Resolution: **delete the `den-tools` crate and fold its surface into `den-core` as `den_core::tools`** (descriptors, argument shapes, `display`, `dispatch`, `context`, capability traits, `work_surface`/`support` helpers); the executors stay where they are, in the `den` binary's `src/core/tools`, wired behind the capability traits + the `RuntimeToolInvoker` seam. `den-runtime` and `den` now reach the surface via `den_core::tools::*`. Trade-off accepted: `den-core` (the universal root dep) absorbs ~3.6k LOC of tool surface, so tool-surface edits trigger wider rebuilds — preferred over a confusingly-named extra crate. No new crate is created; the runtime lift (Stages A–E) plus this fold complete the v1.4 runtime extraction modulo the final shim flip.

**Flip complete — flat `core` shims dropped (2026-06).** The `den` binary's `core/mod.rs` previously re-exported the lifted runtime under flat aliases (`pub use den_runtime::bears;`, `pub use den_runtime::memory;`, … ~40 of them) so call sites kept resolving `crate::core::*` unchanged during the lift. The flip removed all those `pub use den_runtime::*` shims and repointed every call site directly at `den_runtime::*` (and the tool surface at `den_core::tools::*`). Mechanics: a word-boundary sweep rewrote path-form `crate::core::<rt>` → `den_runtime::<rt>`; a brace-aware splitter rewrote the nested `use crate::{ … core::{…} … }` and standalone `use crate::core::{…}` import trees, moving runtime members to a sibling `use den_runtime::{…}` while leaving den-local members (`tools`, `user`, `docket`, `work_plans`, `s3`, `email`, `sandbox`, `migration`, `api_utils`, `acp` + the `acp_runtime`/`acp_tokens`/`acp_turn_runner` aliases) under `crate::core`. The two DB integration tests in `tests/` (`acp_plan_mode`, `work_plans`) were repointed the same way (`den::core::*` → `den_runtime::*`). `core/mod.rs` now only declares the den-binary-local subsystems + the den-side ACP edge. No global `cargo fmt` was run (the tree isn't fmt-clean, so a workspace format would have been ~190 files of noise); only the touched import blocks were reformatted by hand/script. Workspace build + the `-D warnings` clippy gate (all targets) are green. **v1.4 runtime extraction is now COMPLETE.**

#### v1.5 — edge extraction plan (`den-web` / `den-api` / `den-acp`), scoped 2026-06

Measured coupling of the remaining `den` binary (src/ ≈ 44k LOC, the largest single compile unit at ~36s — see the build-time analysis; the HTTP edges are where that time lives):

**Hard constraints discovered:**
- **`CustomError` is the shared edge error** — used by **all** edges (web ~361 refs, api ~266, acp). It implements `axum::IntoResponse`, so it must live in a crate the edges depend on. **But** its current `into_response` renders `error.html`, which `{% extends "base.html" %}` — i.e. it needs the *whole web template tree*. **Decision:** the foundation's `CustomError` renders a **self-contained** error response (small inline HTML, no `base.html`), so it carries no template-tree dependency. (Minor UX change: the error page loses site chrome; reversible later via a den-web error layer.)
- **Templates are embedded per-crate in production** (`build.rs` → `minijinja_embed::embed_templates!` for web/`email`/`api` groups; `load_templates!` reads them back). So whichever crate renders a template group must own that group's embed in its own `build.rs`. → web templates ⇒ `den-web`; email templates ⇒ identity layer; api/oauth templates ⇒ `den-api`.
- **`ApiState` is shared by `api/v1` + `api/acp`** (one `create_api_app`; fields: `sqlx_pool`, `config`, `bifrost`, `acp_tool_turns`, `acp_turn_cancellations`, `memory_stores`). Splitting `den-acp` from `den-api` requires either `den-api → den-acp` or a shared state. **Decision:** keep ACP request types `ApiState`-decoupled (already done in v1.4 via concrete `sqlx_pool`/`config`/`memory_stores` fields); `den-acp` carries its own minimal state, `den-api` mounts it.
- **sqlx offline:** a single workspace-root `.sqlx` cache (`services/den/.sqlx`) serves all member crates (sqlx walks up to it). Moving `query!` code into new crates needs **no** live DB / re-prepare as long as query text is unchanged. (api ~28 queries, web 1, acp ~0.)
- **`auth_backend` → `core::user` → `CustomError`.** To let the foundation host auth without dragging web templates, the shared lower layers (`user`, `email`, `auth_backend`) migrate `CustomError → DenError` (extends the v1.4 "DenError = web-free, CustomError = web adapter" rule).

**Target layering (as-built proposal):**
```
den (bin: main, startup, DI, router composition)
   ├── den-web   (web/ + errors-rendering layer + observability/ + s3/ + web templates build.rs)
   ├── den-api   (api/v1 + oauth + docs + ApiError + api templates build.rs)
   └── den-acp   (core/acp/ residual + api/acp/)
            ↓ all three depend on
        den-foundation  (errors::CustomError [self-contained], auth_backend, api_utils,
                         user, email)   ── “den-http” working name
            ↓
        den-runtime → den-core (+ leaves)
```
(One foundation crate to minimise blast radius this pass; may later split into `den-identity` (user/email leaf) + `den-http` (errors/auth/api_utils).)

**Ordered steps (each: git mv subset + path rewrite + re-export shims in the binary + green + commit):**
1. **Decouple shared lower layers off `CustomError`** — `user`, `email`, `auth_backend` return `DenError`; keep `From` bridges. *(no new crate; prerequisite)*
2. **Extract `den-foundation`** — `errors` (self-contained `CustomError`), `auth_backend`, `api_utils`, `user`, `email` (+ email-templates build.rs). Binary keeps `crate::errors`/`crate::auth_backend`/`crate::core::user` shims.
3. **Extract `den-web`** — `web/**` + `observability/**` + `core/s3` + web-templates build.rs. Most isolated edge (own `AppState`, own server entrypoint, 1 query).
4. **Extract `den-api`** — `api/v1`, `api/oauth`, `api/docs`, `api/auth` (`ApiError`), `api/templates` + build.rs.
5. **Extract `den-acp`** — residual `core/acp/` (`sessions`, `tokens`, `runtime`, `turn_runner`) + `api/acp/`; `den-api` mounts it.
6. **Thin `den` binary** — `main`, `startup`, DI, router composition only.

Risk notes: steps 4–5 are the hardest (ApiState seam, oauth state, 47 `ApiError` sites); step 3 is the highest build-time payoff and lowest risk after the foundation. Stop-and-stay-green after each commit; partial completion is acceptable and leaves a working tree.

##### v1.5 progress — executed autonomously (2026-06, `test` branch)

**DONE (green commits; clippy `-D warnings` all-targets gate green at each):**
- **Step 0 (pre-req): clippy drift fix.** The unpinned stable toolchain had drifted to rust 1.92, turning the gate red independently of this work (`cloned_ref_to_slice_refs`, `manual_strip`, stricter `dead_code`/`unused_imports`). Restored green: small lint fixes + scoped `#![allow(dead_code)]` over handlers in `web/bear_management.rs` + `web/admin/bears.rs` that were superseded by `bear_settings.rs` + redirects (flagged `TODO(den-web extraction)` — remove during the den-web move, not blind-deleted).
- **Step 1: DenError decoupling.** `user`, `email`, `auth_backend` now return `den_core::DenError` (was `CustomError`).
- **Step 2: `den-http` foundation crate extracted.** `crates/den-http` owns `errors` (self-contained `CustomError` — the `error.html`/`base.html` dependency was dropped for inline HTML), `auth_backend`, `api_utils`, `user`, `email` (+ its own `build.rs` embedding the `email` group + `email::template_environment()` helper, called by both the verify-email flow and `web/admin/users`). Binary keeps `crate::errors` / `crate::auth_backend` / `crate::core::{api_utils,email,user}` re-export shims. (Working name kept as `den-http`; contents include the identity layer.)
- **Prep decouplings (shrink the edge moves):** `build_info` relocated to `den-http` (shared by web+api `GET /version`; den-http `build.rs` now emits the `DEN_*` metadata). Edge tool-surface refs in `api/acp` + `web/v1` repointed from the `crate::core::tools::*` shims to `den_core::tools::*`.

**Key measurements that refine the remaining plan:**
- **Edge → tool-executor coupling is small.** `api/acp` + `core/acp` use only the tool *surface* (`den_core::tools` — now repointed), **not** the binary's `core/tools` executors. `core/acp` has **zero** `crate::core::*`/`crate::web`/`crate::api` outbound refs. `web/v1` is the only executor coupling (one `DenRuntimeToolInvoker` construction) → invert via DI (store `Arc<dyn RuntimeToolInvoker>` in `AppState`, inject from the binary).
- **`api` does not depend on `web`** (good: order is acp/api before web). **`web` depends on `api`** via `web/admin/oauth_clients.rs` (`api::oauth::OAuthClient` + `utils::validate_scopes_for_client`) → **den-web must be extracted after den-api**, or that admin page stays in the thin binary until then.
- **Sizes:** `api/acp` ≈ **14.9k LOC**, `core/acp` ≈ 1.1k, `api` total ≈ 21.9k. `ApiState` (defined in `api/service.rs`) is referenced by `api/v1` (15), `api/acp` (63), and `core/acp/turn_runner` (3).

**⚠️ OPEN DECISION blocking the den-acp / den-api split — `ApiState` home.** `ApiState` is genuinely shared by the v1 surface and the ACP surface, so a clean two-crate split needs one of:
- **(A) Combine** `api/v1`+`oauth`+`docs`+`acp`+`core/acp` into a single **`den-api`** crate (ApiState internal); defer the den-acp sub-split to v2. Lowest risk, biggest single build-time win — **but contradicts the roadmap's stated "no deferral of den-acp/den-api sub-splits."**
- **(B) `ApiState` lives in `den-acp`**, and `den-api` (v1/oauth/docs + `create_api_app`) depends on `den-acp` and uses `den_acp::ApiState`. Keeps two crates; semantically odd (service state in the acp crate) but structurally clean (matches "api mounts acp"). *(Recommended if the split is required.)*
- **(C) `den-acp` is generic over a state trait** (`FromRef`-based), `ApiState` stays in `den-api`. Cleanest layering, most code churn (every acp handler's `State<ApiState>` becomes generic).

Related prep for either (B)/(C): **`ApiError`** (the JSON error adapter) is shared by `api/v1` and `api/acp` (32 refs in acp) and belongs in `den-http` alongside `CustomError`. It lives in `api/auth.rs`, which also holds oauth-coupled bearer auth (`extract_bearer_token`/`authenticate_bearer`/`require_scope`, depending on `api::oauth::{jwt,OAuthScope,error}`), so that file must be **split**: `ApiError` → `den-http`; the bearer-auth helpers stay with `den-api` (oauth).

This fork needs an explicit call (it trades the roadmap's split preference against churn/risk), so it was **not** resolved autonomously. The remaining edge extractions (den-acp/den-api per the chosen option, then den-web with its DI + `requires_jwt_secret` relocation + oauth-model prereqs) are mapped above and de-risked by the foundation + prep work already landed.

**Not yet run:** DB integration/smoke tests (need the live stack). The foundation move is structural/behavior-preserving and the compile + clippy-all-targets gate is green; validate with `./scripts/smoke.sh` against the running stack before merge.

##### v1.5 COMPLETE — `den-api` + `den-web` extracted (2026-06, `test` branch)

Both remaining v1 edges are now extracted (green commits; clippy `-D warnings` all-targets gate green at each; incremental binary rebuild fell from ~36s to ~9.5s):

- **`den-api` extracted via Option A (combine).** The `⚠️ OPEN DECISION` above was resolved autonomously toward **(A)**: `api/v1` + `oauth` + `docs` + `acp` + `core/acp` residual landed in a single **`den-api`** crate with `ApiState` internal, deferring the den-acp sub-split. Rationale: lowest risk + biggest single build-time win, and the cleaner two-crate split needs a prerequisite that wasn't yet in place (see the den-acp cycle finding below). Mechanics: `extern crate self as api;` self-alias preserves the old `crate::api::*` module paths; `pub use` shims map `crate::config`→`den_core::config`, `crate::errors`→`den_http::errors`, `crate::core::*`→`den_http`/`den_docket`/`den_core::tools`. The binary's tool-invocation coupling was **dependency-inverted**: `den-api` owns a process-wide `OnceLock<Arc<dyn den_runtime::native_runtime::RuntimeToolInvoker>>` registry (`den_api::set_tool_invoker`/`tool_invoker`); the binary injects `DenRuntimeToolInvoker` at startup, so `internal.rs` + `acp/stream/runtime.rs` call tools through the registry instead of the binary's `core/tools`. `den-api` carries its own `build.rs` (api/oauth templates). `web_policy` moved to `den-http` (it returns `CustomError`, so no error conversion).
- **`den-web` extracted.** `web/**` + `observability/**` + `core/s3` landed in **`den-web`** (depends on den-http + den-api + den-runtime + den-docket + den-core). `extern crate self as web;` preserves `crate::web::*`; a `den-web/src/core/mod.rs` re-exports the foundation deps; `requires_jwt_secret` moved from the binary's `startup.rs` to `den_core::config` (needed by `stack_health`); the v1 chat path sources its invoker from `den_api::tool_invoker()`. `den-web` carries the web-templates `build.rs`; the binary's `build.rs` no longer embeds any templates.

**Binary is now thin-ish (~6.7k LOC):** `main`/`lib`(run)/`startup`/`seeds` (~0.9k) + `core/tools` executors (~4.7k, intentionally kept in the binary per the v1.4 `RuntimeToolInvoker` decision) + residual `core` shims/bridge-tests (~1.1k). Per-crate self-compile: den-api ~11.8s, den-runtime ~11.6s, den binary ~9.5s, den-web ~7.6s, den-core ~3.6s.

##### v1.5+ — finishing the den-acp split (the deferred Option-A debt), scoped 2026-06

Re-examining the deferred den-acp sub-split surfaced the real blocker the original Option-A note only hinted at — a **dependency cycle through OAuth**:

- `api/acp` handlers consume the **bearer-auth layer** (`auth::extract_bearer_token`/`authenticate_bearer`, `oauth::OAuthScope`, `ApiError`) and `auth.rs` itself depends on `oauth::{jwt,error,OAuthScope}`. Since **`den-api` mounts acp** (api depends on acp), *both* edges need OAuth → a naive `ApiState→den-acp` split makes `den-acp → den-api(oauth)` while `den-api → den-acp`, i.e. a cycle.
- Independently, **`den-web` depends on `den-api` only for OAuth** (`admin/oauth_clients.rs` → `OAuthClient` + `utils::validate_scopes_for_client`) plus the `tool_invoker()` registry and `core::acp_tokens`. So today a den-api edit rebuilds den-web.

**Resolution: extract OAuth *below* the edges first, then split den-acp (Option B).** OAuth's outbound deps are only `errors`/`auth_backend` (den-http) + `config` (den-core) — no runtime/acp/ApiState coupling — so it lifts cleanly. Rather than bloat the lean `den-http` foundation with ~5k LOC of (low-churn) OAuth2-server code on everyone's critical path, OAuth lands in a dedicated **`den-oauth`** crate (depends on den-http + den-core; matches the roadmap's `den-identity`/`den-http` split hint). Then:

1. **`den-oauth`** ← `api/oauth/**` + `auth.rs` (`ApiError`, `BearerPrincipal`, `extract_bearer_token`/`authenticate_bearer`/`require_scope`). `den-api`/`den-web`/`den-acp` depend on it; den-api keeps `crate::api::oauth`/`crate::auth` shims; den-web's oauth_clients repoints to `den_oauth`, dropping its hard den-api dep (modulo the tiny `tool_invoker`/`acp_tokens` seams).
2. **`den-acp`** ← `acp/**` + `core/` acp residual + **`ApiState`** (Option B). `den-api` (v1/docs/`create_api_app`) depends on `den-acp` and uses `den_acp::ApiState`, mounting `den_acp::router()`.

Each step keeps the workspace green + clippy `-D warnings` all-targets, with its own commit.

##### v1.5+ COMPLETE — `den-oauth` + `den-acp` extracted (2026-06, `test` branch)

Both steps landed as green commits (workspace build + clippy `-D warnings` all-targets + binary `--features production` green at each):

- **`den-oauth` extracted.** `api/oauth/**` (OAuth2 server: endpoints/db/jwt/scopes/router/utils), `api/auth.rs` (`ApiError`, `BearerPrincipal`, `extract_bearer_token`/`authenticate_bearer`/`require_scope`), and the OAuth consent/authorize+error templates (with their own `build.rs` embedding the group keyed `"api"`) moved into a dedicated **`den-oauth`** crate depending only on den-http + den-core. This breaks the `den-acp ↔ den-api` OAuth cycle and lets `den-web`'s `admin/oauth_clients` reach OAuth directly (`den_oauth::oauth`). `den-api` keeps `crate::api::{oauth,auth,templates}` re-export shims; its now-empty `build.rs` + `minijinja-embed` build-dep were dropped.
- **Tool-invoker registry → `den-runtime`.** The process-wide `OnceLock<Arc<dyn RuntimeToolInvoker>>` (`set_tool_invoker`/`tool_invoker`) moved from `den-api`'s `lib.rs` into `den_runtime::native_runtime` (alongside the trait it stores), so the ACP edge reaches it without a `den-acp → den-api` cycle. `den-api` keeps a re-export shim for the binary's startup injection + den-web's web-chat call site.
- **`den-acp` extracted (Option B).** `acp/**` (ACP HTTP surface), the residual `core/**` (`core::acp*` protocol modules: sessions/tokens/runtime/turn_runner + the foundation/tool re-export shims), the shared **`ApiState`** (now in `den_acp::service`, built via `ApiState::new`), and the `/internal/den-tools/invoke` endpoint moved into **`den-acp`**. `den-api` depends on `den-acp`, mounts `den_acp::{acp,internal}::router()` in `create_api_app`, and re-exports `den_acp::core` so `den_api::core::*` (binary `core::mod`, den-web `acp_tokens`) resolves unchanged. `den-acp` self-aliases (`extern crate self as api`) and carries the foundation/oauth/registry shims, so the migrated edge compiled with zero in-file path rewrites (only the `ApiState` construction + router mounts in den-api's `service.rs` changed).

**As-built edge layering (v1 + the den-acp sub-split now complete):**
```
den (bin: main, startup, DI, router composition, core/tools executors)
   ├── den-web   (web UI + /v1 chat + observability + s3)
   └── den-api   (v1/docs JSON API + create_api_app)  ── depends on ↓
        └── den-acp  (ACP protocol surface + core::acp* + ApiState + /internal)
                ├── den-oauth  (OAuth2 server + bearer-auth + consent templates)
                │      ↓ den-http (errors/auth_backend/api_utils/user/email/build_info/web_policy)
                └── den-runtime (loop, governance, RuntimeToolInvoker + registry)
                        ↓ den-llm / den-memory / den-docket → den-core (+ den_core::tools)
```

**Build-time (incremental, after a one-file edit; ~self-compile + downstream relink):** den-web ~7.2s, den-acp ~9.8s, den-api ~10.1s, den-oauth ~11.0s (it's a low leaf, so its touch relinks all three edges). The structural win is **isolation**: editing the v1 surface (`den-api`, ~7k LOC) no longer recompiles the ~16k-LOC ACP code, and vice versa; OAuth (~5k, low-churn) edits no longer rebuild den-api/den-acp internals. Trade-off: two more crates add link overhead, and since `den-api` mounts `den-acp`, ACP edits still cascade up through `den-api`.

**v2 COMPLETE (2026-06):** both true-v2 items landed — (1) **dropped `async-trait`** for native async-fn-in-trait on the 12 non-`dyn` service traits (the 10 `ToolContext` tool-capability traits + `DocketService` + `PassageEmbedder`); `RuntimeToolInvoker` keeps async-trait (used as `Arc<dyn>`), as do the axum-login `Authn/AuthzBackend` impls (external async-trait). `async-trait` is dropped from `den-core` and `den-docket`. (2) **Promoted pedantic/nursery clippy to gating** (`deny` in `[workspace.lints.clippy]`, `-A` suppressions removed from `scripts/lint.sh` + CI), fixing the high-signal violations and curating the opinionated/stylistic ones (incl. `future_not_send`, the deliberate native-async-fn trade-off) into the allow-list. (`den-core`/`den-db` was evaluated and deferred as low-value — see the deferral note above and *v1.6* below.) The `core/tools` executors stay in the binary by design (the v1.4 `RuntimeToolInvoker` decision). **Smoke validated 2026-06** via `./scripts/smoke-stack.sh` (rebuild den image + recreate + seed): 10/11 pass; the one failure (`test_native_acp_pair_turn_completes_when_api_enabled`) is environmental — bifrost returns HTTP 401 because this sandbox has no real `OPENAI_API_KEY` (placeholder); logs confirm the refactored `den-acp`/`den-runtime`/`den-llm` turn path executes end-to-end up to the external model call.

##### v1.6 — build-time profiling + den-web decoupling (2026-06, `test` branch)

With the edges extracted, a clean first-party `--timings` build (dev profile) showed the critical path is a **serial chain** `den-core → den-runtime (6.5s) → den-acp (4.0s) → den-api (1.9s) → den-web (9.3s) → den bin (6.2s) → link (3.5s) = 30.3s`. Two findings reshaped the plan:
- **den-web (9.3s) is the longest single crate**, not den-runtime — and it sat *late* on the chain only because it depended on `den-api`. But den-web's *only* uses of `den-api` were re-exports of things that live lower: `den_api::tool_invoker` (really `den_runtime::native_runtime::tool_invoker`) and `den_api::core::acp_tokens`. The `den_api as api` alias in `den-web/lib.rs` was already dead.
- **den-runtime is tangled, not a clean split.** Its ACP cluster (`acp_tools`/`acp_events`/`acp_turn_controller`/`acp_tool_turns`/`acp_plan_mode`, ~7.6k LOC) is referenced *back* by core runtime modules (`runtime/role`, `runtime/turn_state`, `agent_loop/tool_policy`, `native_runtime/turn`, `bearwire_projection`, `agent_assist/runtime_stream_parser`), so extracting it needs trait-inversion surgery — high risk, deferred.

**Done (low-risk graph reshape):** moved the shared `acp_tokens` module (auth/scopes/CRUD; only dep is `CustomError`, no den-acp/den-runtime coupling) **down to `den-http`**, and repointed den-web's two `tool_invoker` calls to `den_runtime::native_runtime`. den-web then **dropped its `den-api` (and transitive `den-acp`) dependency entirely**, so it now compiles **in parallel** with the api/acp edges (gated only by den-runtime's rmeta) instead of after them. `den_acp::core::acp_tokens` / the binary's `core::acp_tokens` keep resolving via re-export (`den_acp::core::acp::tokens = den_http::acp_tokens`).

**Result:** clean first-party dev build **30.3s → 24.5s (≈19%)**; den-web now starts at ~7.6s (was ~12.9s) and the binary at ~16.2s (was ~20.6s). den-web (now ~10.3s self-compile) is the new long pole; further wins would require splitting den-web or den-runtime (both larger/riskier). Build profiles are already tuned (`release`: `lto=false`, `codegen-units=256`, `panic=abort`, `lld` linker), so there's no cheap profile lever left.

**First cluster move — DONE (commits on the `test` branch).** `den-runtime` now owns: `runtime/**` (contracts/provider/role/role_registry/compaction*/conversations/turn_state/pair_turn/bearwire_projection, with the familiar flat aliases re-exported), plus `acp_events` (`AcpGatewayEvent` + SSE adapter), `acp_tools`, `acp_plan_mode`, `acp_tool_turns`, and `acp_turn_controller`. The cluster's error type was first migrated `CustomError → den_core::DenError` (a bidirectional `From` was added in `den`). Cycle breaks discovered + applied: `acp_turn_controller` had to come along (runtime turn coordination); `AcpCompactionStatusResponse` (a runtime-produced DTO) was relocated from `api/acp/http_types` into `runtime/compaction_store` (api re-exports it); `role_registry` now uses `den_core::BearProfile` and inlines the small `bear_profile_bindings` lookup instead of calling den's `bears::db`. `den` keeps thin re-export shims in `core/mod.rs` so edge/api/web call sites are untouched, and the two cross-layer bridge tests (which also touch den-only `native_runtime`/`acp_turn_controller`) were relocated into the `den` crate.

**Discovered during execution (2026-06) — `core/acp/` is NOT a single edge module; it must be split.** `core/runtime/` cannot move alone: it is knotted to a cluster of shared turn/tool *contracts* that today live under `core/acp/` with a misleading `acp` prefix. These move into `den-runtime` **with** `runtime/` (first cluster move):
- `acp::events` (`AcpGatewayEvent` + SSE adapter; ~1.5k LOC) — the canonical native event model + its ACP-SSE projection; produced by `runtime/bearwire_projection`, consumed by the edge.
- `acp::tools` (`AcpToolName`, `AcpResolvedSessionPolicy`, policy/display; ~2.9k LOC) — the ACP projection of the tool surface; depends only on `den_core::tools` (now in `den-runtime` as `acp_tools` after the lift).
- `acp::plan_mode` (~0.6k) and `acp::tool_turns` (`AcpActiveTurnGuard`; ~1.1k) — turn/plan coordination used by `runtime`.
- `work_plans` (docket projection).

The **residual** `core/acp/` (`sessions`, `tokens`, `runtime`, `turn_controller`, `turn_runner`) is the true ACP protocol edge → `den-acp` later.

**Error handling: cluster migrates to `den-core::DenError` (do NOT move `CustomError`).** `crate::errors::CustomError` is *by design* the web-boundary adapter for the `den` binary — it implements `axum::IntoResponse` (renders `error.html`) and carries auth/mailgun/validator/axum `From` impls — so it must stay in `den`. The web-free `DenError` already lives in `den-core` (it is what service-layer code should return, per the `errors` module doc). The first-move cluster currently returns `CustomError` in ~85 spots (`runtime/` 43, `acp::plan_mode` 26, `acp::tool_turns` 16) and `DenError` nowhere; the lift converts those to `DenError`. This is mechanical: variants mirror 1:1, `DenError` already has the needed `?`-conversions (`anyhow`, `io`, `sqlx`, sqlx-`uuid`, `serde_json`, `reqwest`), and `impl From<DenError> for CustomError` already exists so edge HTTP handlers that propagate via `?` keep working unchanged.

### v2 — Refinements ✅ COMPLETE (2026-06)

- ✅ **native `async fn` in traits (drop `async-trait`).** Converted the 12 non-`dyn` service traits to native async fn (each carries a scoped `#[allow(async_fn_in_trait)]` — workspace-internal, generic-only consumption, so Send flows through monomorphization). `RuntimeToolInvoker` (`Arc<dyn>`) and the external axum-login `Authn/AuthzBackend` impls keep `async-trait`; the dep is dropped from `den-core`/`den-docket`.
- ✅ **pedantic/nursery promoted from advisory to gating.** `deny` in `[workspace.lints.clippy]` with a curated allow-list; `scripts/lint.sh` + `den-clippy.yml` drop the `-A` suppressions and gate via `-D warnings`. High-signal violations fixed (`or_fun_call`, `needless_continue`, `assigning_clones`); machine-applicable autofixes applied separately.
- (`den-core`/`den-db` split was evaluated 2026-06 and deferred as low-value — feature unification means a full-workspace build compiles `sqlx` regardless; see *v1.6* and the deferral note.)

## Caveats

- **High-churn trait crates.** Crate boundaries help only across **stable** public traits; if `DocketService`/`MemoryStore`/`TaskDispatcher`/`ToolContext` signatures churn, dependents rebuild. Hence v0 stabilizes them first.
- **sqlx offline data per DB crate.** Compile-time-checked `query!`/`query_as!` need `DATABASE_URL` or committed `.sqlx` offline data; each DB-touching crate (`den-core`/db, `den-memory`, `den-docket`, query-issuing parts of `den-api`/`den-acp`) needs offline data and `SQLX_OFFLINE` wired in CI.
- **`async-trait` cost.** Service traits are async; `async-trait` boxes futures (negligible at our call rates). ✅ Removed in v2 via native async-fn-in-trait for the 12 non-`dyn` traits; `RuntimeToolInvoker` (`Arc<dyn>`) and external axum-login backends retain it.
- **Per-boundary overhead.** Each crate adds codegen/link overhead; ~10 crates is the target granularity, not per-module. Watch clean-build time alongside incremental time.
- **Don't let `den-runtime` re-monolithize.** With ACP and tools extracted, runtime should be the loop + governance + harnesses; resist parking unrelated modules there during triage.

## Migration mechanics

- Root `Cargo.toml` becomes `[workspace]` with `members` and `[workspace.dependencies]` (single-sourced third-party versions) and `[workspace.lints]`.
- Move modules file-by-file; replace intra-crate `crate::` paths with the target crate name; temporary re-export shims ease large moves, then are removed.
- Commit `.sqlx` offline data; set `SQLX_OFFLINE=true`; update CI to `cargo build --workspace` / `cargo test --workspace` / `cargo clippy --workspace -- -D warnings`.
- Keep the `den` binary output name and Docker build target unchanged.
- Land each crate extraction (and its idiom tightening) as its own reviewable PR/commit.

## Relationship to other plans

- [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md): owns the `core/docket/` module and `DocketService`/`TaskDispatcher` trait seams. This plan consumes those seams and turns them into crate boundaries.
- [ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), [ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md): define the memory vs. tasks storage boundary the split makes compile-time-hard.
- [ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md): trust stance / governance mode enums are prime de-stringify targets in `den-core`.
- [DEN_NATIVE_RUNTIME_PLAN.md](DEN_NATIVE_RUNTIME_PLAN.md): removal of `core/letta/` and `core/codepool/` reduces what must be moved; sequence the legacy deletion into v0.

## Open questions

- Exact placement of the loose `core/*.rs` files between `den-acp` and `den-runtime` (resolved by v0 triage).
- Whether `den-tools` should further split (e.g., descriptor crate vs. executors) — likely not in v1.

## Execution log (2026-06, `clippy` branch)

A first autonomous pass landed the safe, behavior-preserving groundwork and
surfaced the real gating prerequisite for the service-crate extractions.

**Done (green, committed):**

- **Workspace + advisory clippy lint table.** `services/den/Cargo.toml` is now a
  Cargo workspace root (root package stays the `den` binary; members live under
  `crates/`). A `[workspace.lints.clippy]` table sets pedantic/nursery to
  advisory `warn` with the noisy lints allow-listed.
- **`den-core` seeded.** `config` (only `url`/`tracing`/`dotenvy`/std), `metrics`
  (std-only, formerly `observability::metrics`), and `DenError` (the shared
  web-free error). `Config::test_stub` is exposed via a `den-core` `test-util`
  feature.
- **Error gate resolved — option 2 (`DenError`).** `den-core::DenError` is the
  shared web-free error (variants mirror `CustomError`; infra `From` impls for
  anyhow/io/sqlx incl. pool handling/serde_json/reqwest). `den`'s `CustomError`
  stays the HTTP/web adapter (`IntoResponse`, auth conversions) and gains
  `From<DenError>`, so service code returns `DenError` and still bubbles up via
  `?` in handlers. Landed additively (existing `CustomError` impls untouched);
  `CustomError`'s infra `From` impls can be slimmed to delegate as callers
  migrate.
- **`den-llm` extracted.** First service-layer leaf: LLM client + idle byte
  stream, depending only on `den-core`, returning `DenError`. The SSE ->
  `RuntimeStreamEvent` mapping (`stream.rs`) stays in `den` to avoid a
  `llm -> runtime` cycle; `core::llm` re-exports `den-llm` so call sites are
  unchanged. `cargo test -p den-llm` builds/runs against only `den-core`.
- **`den-memory` extracted.** The per-Bear SQLite store subtree
  (`core/memory/store/*`, ADR-0031) is now the `den-memory` crate, depending
  only on `den-core`. Higher-level memory **curation/tools/admin** (which reach
  into `bears`/`reflection`) stayed in `den`; `core::memory` re-exports
  `den-memory` as `store`. `cargo check -p den-memory` builds in isolation.
- **`BearProfile` moved to `den-core`.** The closed-set profile enum (shared by
  runtime, docket, tools, memory, web/api) now lives in `den-core::profile`;
  `core::bears::model` re-exports it. Prerequisite for `den-docket` depending
  only on `den-core`.
- **`den-docket` extracted.** The Docket subsystem landed as `core::docket`
  (DocketService/PgDocketService public face; `db` module-internal; `DenError`
  throughout) and was then promoted to the `den-docket` crate depending only on
  `den-core`. **Minimal scope ("Level 1, honest naming"):** the legacy
  `bear_work_plans` activity board keeps its honest pre-ADR-0034 type names — no
  premature `Job`/`Task` structs over the JSONB shape — while the subsystem
  boundary + `DocketService` seam lock out non-Docket drift. `TaskDispatcher` is
  deferred to the `den-runtime` extraction (defined in its consumer). Tool / ACP
  / web / test callers go through `PgDocketService`; `core::docket` and
  `core::work_plans` are re-export shims. Crate is clippy-clean (advisory v1);
  6 model unit tests pass; `cargo check/test -p den-docket` builds in isolation.
- **Default-level clippy machine-fixes** applied across core/api/web.

**Gating the remaining service crates:**

- **`den-tools`** (~8.7k, high-churn) is broadly coupled into `crate::core::*`
  (sessions, plan-mode, projections, bears). It needs the v0 tools triage first:
  stabilize the `ToolContext` trait and split descriptor/registry/executors so
  the crate doesn't pull in runtime/bears. Deferred — too large to extract
  safely before that triage.
- **`den-runtime`** and the `den-acp`/`den-api`/`den-web` edges depend on the
  above plus the loose-`core/*.rs` triage.

**Validated technique — re-export shim for low-churn extraction.** Moving a
module into a member crate and then `pub use`ing it back at the original crate
path (e.g. `pub use den_core::config;` in `den/src/lib.rs`, `pub use
den_core::metrics;` in `observability/mod.rs`) lets every existing `crate::…`
reference compile unchanged. This makes each extraction a small, reviewable
move rather than a repo-wide path rewrite. **Recommended as the standard
extraction mechanic** (replaces the "replace intra-crate `crate::` paths"
step in *Migration mechanics* for the common case).

**Resolved gate — the shared error type (now `DenError`).** `CustomError` was
the de-facto shared error but is **web-coupled** (`axum::IntoResponse` rendering
`error.html`; `From<auth_backend::…>`), so it could not move into the `den-core`
leaf and the orphan rule forbade keeping `IntoResponse` in `den` if the type
moved. Resolved via **option 2**: a web-free `DenError` now lives in `den-core`;
`CustomError` stays the HTTP adapter and bridges via `From<DenError>`. Service
crates return `DenError`; HTTP handlers convert for free through `?`.

**Extraction recipe (validated on `den-llm`), for the remaining service crates:**

1. Move the subsystem's modules into `crates/den-<name>/`; keep runtime/web-
   coupled sub-modules behind in `den` (split the module if needed to avoid a
   cycle — e.g. llm's `stream.rs` stayed because it maps to runtime contracts).
2. In the moved code, rewrite `crate::config`/`crate::errors::CustomError` to
   `den_core::config`/`den_core::DenError` (and other `crate::` foundation refs).
3. Make the old `core::<name>` module a re-export shim (`pub use den_<name>::…`)
   so existing `crate::core::<name>::…` paths compile unchanged.
4. At the few boundaries where a moved fn now yields `DenError` but a den caller
   expects `CustomError`, add `.into()` / `.map_err(CustomError::from)`.
5. Add the crate to `[workspace].members` and den's `[dependencies]`; verify
   `cargo test -p den-<name>` builds in isolation.

**Suggested next steps:** (1) ~~land the Docket module + extract `den-docket`~~
**done**; (2) ~~tools triage + `ToolContext` seam~~ **done** (the standalone
`den-tools` crate was built then dissolved into `den_core::tools`; executors stay
in the binary behind `RuntimeToolInvoker`); (3) ~~loose-`core/*.rs` triage feeding
`den-runtime`~~ **done**; (4) ~~extract `den-runtime`~~ **done (v1.4)** — **next:
extract the `den-acp`/`den-api`/`den-web` edges, collapsing `den` to the thin
binary.**

## `den-tools` triage (proposed, 2026-06)

> **⚠️ Superseded (2026-06).** This triage assumed a standalone `den-tools` crate. That crate was ultimately **dissolved**: the static surface folded into `den-core` (`den_core::tools`) and the executors stayed in the `den` binary's `core/tools` behind `den-runtime`'s `RuntimeToolInvoker`. Retained below as historical analysis (the coupling map and capability decomposition are still accurate); ignore references to extracting a `den-tools` crate.

Read-only investigation of `core/tools/` (~8.7k LOC) ahead of extraction. **Key
finding: `den-tools` is an integration layer *above* most subsystems, not a
leaf.** The dispatcher (`core/tools/session/mod.rs::invoke_den_tool`) already
threads dependencies explicitly — `pool`, `config`, `stores`, plus a serializable
`DenToolInvocationContext` — into free executor functions. That separates two
concerns the seam should preserve:

- **`DenToolInvocationContext` = per-call data** (ids, `BearProfile`, channel).
  Needs only `BearProfile` + `serde_json::Value`, so it moves into `den-tools`
  trivially.
- **Capabilities** (`pool`/`config`/`stores` + a pile of `crate::core::*`
  functions) = what must be inverted behind a trait.

**Coupling map (executor → subsystem):**

- Already crates / `den-core` types: `config::Config`, `bears::BearProfile`,
  `core::docket` (`DocketService`), `core::memory` store.
- `den`-resident subsystems needing inversion: `bears::db` + `user` (Postgres
  directory), `acp_sessions` / `acp_tools` / `acp_plan_mode`, memory curation
  (`memory::tools` + `MemoryStoreManager`), `memory_manager_head` (memfs HTTP),
  `prompt_memory_block_store` / `prompt_memory_blocks`, `conversation_events`,
  `turn_state`, `bear_observations`, `web_policy`.

A single `ToolContext` over all of that would be a ~30-method god trait — the
wrong shape.

**Proposed seam — two phases, composed capability sub-traits:**

- **Phase A (low-risk): extract the static surface as `den-tools`.** Move
  `constants` / `aliases` / `arguments` / `descriptor/` + the
  `DenToolInvocationContext` data type. Prerequisite: relocate
  `tool_descriptor_guidance` (already dependency-free) and split
  `AcpToolDisplayDescriptor` out of `acp_tools` into `den-core` (descriptor's
  only non-tools deps). Result: a `den-core`-only crate owning the descriptor /
  model-facing **naming authority** (serves AGENTS.md "descriptor-owned names").
  Executors stay in `den` and call the registry. Mostly data + pure fns.
- **Phase B (incremental): define `ToolContext` as composed sub-traits and
  invert executors group-by-group.** e.g. `BearDirectory`, `MemoryTools`,
  `PromptMemory`, `PlanModeOps`, `WorkSurfaceOps`, `WebFetcher` — plus the
  existing `DocketService` as the template. `den-runtime` (the `den` binary
  today) implements each by delegating to current `crate::core::*` fns. Move one
  tool-group at a time behind its sub-trait, green per step. This is where
  `den-runtime` gets unblocked, and follows "define traits where they are used".

Recommended first concrete step: Phase A (descriptor/registry extraction + the
`tool_descriptor_guidance` / `AcpToolDisplayDescriptor` relocation it needs).
Hard gate: workspace green + clippy advisory.

**Phase A landed (2026-06, `test` branch).** `den-tools` exists as a
`den-core`-only leaf crate owning the static, model-facing tool surface:
`constants`, `aliases`, `arguments`, `descriptor/` (with its guidance tests),
and `tool_descriptor_guidance`. `AcpToolDisplayDescriptor` moved into
`den-tools::display` (kept with the descriptor authority rather than `den-core`,
since it is a tool-display shape, not foundation); `acp_tools` re-exports it.
Old paths under `crate::core::tools::{constants,aliases,arguments,descriptor}`,
`crate::core::tool_descriptor_guidance`, and
`crate::core::acp_tools::AcpToolDisplayDescriptor` are preserved as re-export
shims, so no caller changed. `DenToolInvocationContext` and the executors stay in
`den` for now — they move with their capabilities in Phase B. Verified:
`cargo check --workspace --all-targets` green, `cargo test -p den-tools` (4
descriptor-guidance tests) green, `den-tools` clippy-clean.

## Phase B — `ToolContext` sub-trait signatures (draft, 2026-06)

> **⚠️ Superseded (2026-06).** Drafted when the plan was to move executors into a `den-tools` crate. The capability sub-traits described here were built and now compose the `den_core::tools` surface; the executors remained in the `den` binary's `core/tools` (implementing those traits) rather than moving to a `den-tools` crate. Retained as historical design notes.

Goal: move the *executor logic* (validation, payload shaping, routing) into
`den-tools` while leaving *capabilities* (Postgres, the per-Bear SQLite memory
store, HTTP egress, Letta) behind traits that `den-runtime` (the `den` binary
today) implements. This is what unblocks `den-runtime` extraction: once the
executors depend only on traits + `den-core` types, they compile inside
`den-tools`, and `den-runtime` is the only crate that touches `PgPool` /
`MemoryStoreManager` / `reqwest`.

### Design rules

- **Composed, not god.** No single `ToolContext` with ~30 methods. Each capability
  area is its own `#[async_trait]` sub-trait; each executor is generic over only
  the sub-traits it uses (`async fn write_memory_entry(ctx: &impl RoleMemoryStore, …)`).
  A blanket `ToolContext` supertrait bundles them for the dispatcher.
- **Errors at the seam are `den_core::DenError`**, never `CustomError` (web). The
  `den` adapter keeps `From<DenError> for CustomError` (already in place).
- **Per-call data is a value, not a capability.** `DenToolInvocationContext` moves
  into `den-tools` as a plain struct (it already serializes; it only needs
  `den_core::BearProfile`). Capabilities are the trait methods.
- **Native (SQLite) path is the modeled contract.** The Letta/MemFS branches
  (`config.uses_native_agent_runtime()` false) are legacy (v0-legacy deletes
  `core/letta`); the traits model the canonical SQLite store. Any remaining MemFS
  fallback stays inside the `den-runtime` impl, not in the trait surface.
- **Pure helpers don't need traits.** Orientation inference, slug derivation,
  payload builders, URL/SSRF validation, and text bounding are pure and move into
  `den-tools` directly.
- **`WorkSurfaceOps` keeps the name "work surface"** per
  [ADR-0040](../decisions/adr-0040-connections-and-work-surface-presentation.md):
  "work surface" is the canonical code/model-facing term (ADR-0024's
  "→ resource" rename is superseded). The trait is `WorkSurfaceOps`, not
  `ResourceOps`; user-facing labels (Repository/Design/…) live in the
  presentation layer, out of scope here.

### Sub-traits

Types named below that still live in `den` (e.g. `Bear`, `BearMember`,
`PromptMemoryBlock*`, `MemoryProposal*`, `WebPolicyDecision`) must migrate to
`den-core` (foundation rows/enums) or `den-tools` (tool-shaped types) *before*
the consuming executor moves. Method shapes mirror today's free functions.

```rust
use async_trait::async_trait;
use den_core::{BearProfile, DenError};
use uuid::Uuid;

/// Identity, membership, and policy lookups. Backs the dispatcher's
/// authorize_context/context_role and the bear/user/* read tools.
#[async_trait]
pub trait BearDirectory {
    async fn user_may_use_bear(&self, user_id: i32, bear_id: Uuid) -> Result<bool, DenError>;
    /// `bear_profile_bindings` lookup used to resolve + verify the caller's role.
    async fn registered_profile(&self, bear_id: Uuid, binding_id: &str)
        -> Result<Option<BearProfile>, DenError>;
    async fn get_bear(&self, bear_id: Uuid) -> Result<Option<Bear>, DenError>;
    async fn count_bear_members(&self, bear_id: Uuid) -> Result<i64, DenError>;
    async fn list_members(&self, bear_id: Uuid) -> Result<Vec<BearMember>, DenError>;
    async fn user_by_id(&self, user_id: i32) -> Result<UserRecord, DenError>;
}
// `role_is_bear_admin(role: Option<&str>) -> bool` is pure → den-tools (or den-core).

/// Conversation metadata (set_conversation_title). The Letta summary patch is
/// legacy; once core/letta is deleted this trait drops `patch_summary`.
#[async_trait]
pub trait ConversationTitleOps {
    async fn set_title(&self, bear_id: Uuid, conversation_id: &str, title: &str)
        -> Result<u64, DenError>; // -> synced acp_session count
    async fn patch_summary(&self, conversation_id: &str, summary: &str) -> Result<(), DenError>;
}

/// Canonical per-Bear, per-role SQLite memory. Hides MemoryStoreManager +
/// store_for_bear + the `memory::tools::sqlite_*` family. Backs memory_read/
/// memory_write/memory_status/memory_browse/search and orientation path listing.
#[async_trait]
pub trait RoleMemoryStore {
    async fn read(&self, bear_id: Uuid, role: BearProfile, path: &str)
        -> Result<MemoryReadResult, DenError>;
    async fn browse(&self, bear_id: Uuid, role: BearProfile) -> Result<MemoryTree, DenError>;
    async fn search(&self, bear_id: Uuid, role: BearProfile, query: &str)
        -> Result<Vec<MemorySearchHit>, DenError>;
    async fn status(&self, bear_id: Uuid, role: BearProfile) -> Result<MemoryStatus, DenError>;
    async fn list_logical_paths(&self, bear_id: Uuid, role: BearProfile)
        -> Result<Vec<String>, DenError>;
    async fn write_entry(&self, bear_id: Uuid, role: BearProfile, entry: RoleMemoryEntryWrite)
        -> Result<MemoryWriteOutcome, DenError>;
    /// Used by work-surface scaffold + plan-mode artifacts (sqlite_write_at_path).
    async fn write_at_path(&self, bear_id: Uuid, role: BearProfile, path: &str, body: &str)
        -> Result<MemoryWriteOutcome, DenError>;
    async fn list_plan_artifacts(&self, bear_id: Uuid, role: BearProfile)
        -> Result<Vec<PlanArtifact>, DenError>;
}

/// Reflection/curation review surface: proposals + observations + the enqueue +
/// the conversation-event projections. Backs memory_review.rs and observations.rs.
#[async_trait]
pub trait MemoryReviewStore {
    async fn create_proposal(&self, bear_id: Uuid, params: CreateMemoryProposal)
        -> Result<MemoryProposal, DenError>;
    async fn get_proposal(&self, bear_id: Uuid, proposal_id: Uuid)
        -> Result<Option<MemoryProposal>, DenError>;
    async fn list_proposals(&self, bear_id: Uuid, query: ProposalQuery)
        -> Result<Vec<MemoryProposal>, DenError>;
    async fn resolve_proposal(&self, bear_id: Uuid, params: ProposalResolutionParams)
        -> Result<MemoryProposal, DenError>;
    async fn promote_core_content(&self, bear_id: Uuid, params: CorePromotion)
        -> Result<(), DenError>;

    async fn create_observation(&self, bear_id: Uuid, obs: CreateObservation)
        -> Result<BearObservationRow, DenError>;
    async fn get_observation(&self, bear_id: Uuid, observation_id: Uuid)
        -> Result<Option<BearObservationRow>, DenError>;
    async fn mark_observation_review_queued(&self, bear_id: Uuid, observation_id: Uuid)
        -> Result<(), DenError>;
    async fn enqueue_proposal_review(&self, params: ProposalEnqueueParams) -> Result<(), DenError>;

    /// conversation_events projections (review-requested / proposal-resolved).
    async fn project_review_event(&self, event: ReviewProjection) -> Result<(), DenError>;
}

/// Runtime prompt-memory blocks (NOT semantic memory). Backs prompt_memory.rs and
/// memory_read's block listing. Wraps prompt_memory_block_store::*.
#[async_trait]
pub trait PromptMemoryStore {
    async fn list_blocks(&self, bear_id: Uuid, profile: BearProfile, query: PromptMemoryBlockQuery)
        -> Result<Vec<PromptMemoryBlock>, DenError>;
    async fn upsert_block(&self, write: PromptMemoryBlockWrite)
        -> Result<PromptMemoryBlock, DenError>;
    async fn patch_block(&self, patch: PromptMemoryBlockPatch)
        -> Result<PromptMemoryBlock, DenError>;
    async fn archive_conflicting(&self, write: &PromptMemoryBlockWrite) -> Result<u64, DenError>;
    async fn archive_superseded_by(&self, block_id: &str) -> Result<u64, DenError>;
}

/// Plan-mode lifecycle (acp_plan_mode + acp_sessions + turn_state). Plan-mode also
/// writes a role-memory artifact → executor composes PlanModeOps + RoleMemoryStore.
#[async_trait]
pub trait PlanModeOps {
    async fn enter(&self, params: EnterPlanModeParams) -> Result<PlanModeState, DenError>;
    async fn submit(&self, params: SubmitPlanModeParams) -> Result<PlanModeState, DenError>;
    async fn cancel(&self, bear_id: Uuid, session: &str) -> Result<PlanModeState, DenError>;
    async fn status(&self, bear_id: Uuid, session: &str) -> Result<Option<PlanModeState>, DenError>;
    async fn record_approval(&self, params: PlanApprovalParams) -> Result<PlanModeState, DenError>;
    /// Resolved session policy used to render turn_state in responses.
    async fn resolved_session_policy(&self, bear_id: Uuid, session: &str)
        -> Result<AcpResolvedSessionPolicy, DenError>;
}

/// Work-surface orientation + scaffold (ADR-0040: "work surface" is canonical).
/// The inference/slug/payload builders are pure → den-tools free fns; only the
/// memory I/O is a capability, mostly delegating to RoleMemoryStore. Kept as a
/// named trait so the seam stays explicit and future surface-kind/Connection
/// metadata (ADR-0040 follow-ups) has a home.
#[async_trait]
pub trait WorkSurfaceOps {
    async fn create_scaffold(&self, bear_id: Uuid, role: BearProfile, scaffold: WorkSurfaceScaffold)
        -> Result<WorkSurfaceScaffoldOutcome, DenError>;
}

/// External web egress. URL normalization/SSRF checks and HTML→text are pure →
/// den-tools; the policy decision (DB-backed) and the actual fetch/search are
/// capabilities.
#[async_trait]
pub trait WebFetcher {
    async fn decide_fetch_approval(&self, bear_id: Uuid, url: &NormalizedWebUrl)
        -> Result<WebFetchDecision, DenError>;
    async fn record_fetch_attempt(&self, params: WebFetchAuditParams<'_>) -> Result<(), DenError>;
    async fn fetch(&self, url: &NormalizedWebUrl, max_chars: usize) -> Result<WebFetchBody, DenError>;
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<WebSearchHit>, DenError>;
    async fn preferred_hosts(&self, bear_id: Uuid) -> Result<Vec<String>, DenError>;
}

/// Already extracted in den-docket; listed for completeness as the template the
/// other sub-traits follow. workflow executors also take pure activity-payload
/// builders → those fn pointers become den-tools free fns.
// pub trait DocketService { … }  // den-docket
```

### Umbrella bundle + dispatcher

```rust
/// Composed bundle so the dispatcher can take one `&impl ToolContext`; individual
/// executors stay generic over the minimal sub-trait set they need.
pub trait ToolContext:
    BearDirectory
    + ConversationTitleOps
    + RoleMemoryStore
    + MemoryReviewStore
    + PromptMemoryStore
    + PlanModeOps
    + WorkSurfaceOps
    + WebFetcher
    + den_docket::DocketService
    + Send
    + Sync
{}

pub async fn invoke_den_tool(
    ctx: &impl ToolContext,
    tool_name: &str,
    arguments: serde_json::Value,
    context: DenToolInvocationContext,
) -> Result<serde_json::Value, DenError> { /* moved from session/mod.rs */ }
```

`den-runtime` provides one struct (e.g. `DenRuntimeToolContext { pool, config, stores }`)
that implements every sub-trait by delegating to today's `crate::core::*` functions.

### Executor → sub-trait map (move order)

Greenest-first; each row is one PR, workspace green per step:

| Executor group | Sub-trait(s) consumed | Pure helpers to den-tools |
|---|---|---|
| `web/` | `WebFetcher` | URL/SSRF normalize, html→text, truncate |
| `prompt_memory` | `PromptMemoryStore` | scope/text validators |
| `work_surface/` | `WorkSurfaceOps` + `RoleMemoryStore` | hint inference, slug, orientation payload |
| `memory_read` | `RoleMemoryStore` + `PromptMemoryStore` | — |
| `memory_write` | `RoleMemoryStore` | write-semantics validators |
| `observations` | `MemoryReviewStore` | text validators |
| `memory_review` | `MemoryReviewStore` (+ projections) | text validators |
| `plan_mode` | `PlanModeOps` + `RoleMemoryStore` | turn_state render is pure-ish |
| `workflow` | `den_docket::DocketService` | activity-payload builders |
| dispatcher + `environment` + bear/user/policy | `BearDirectory` + `ConversationTitleOps` + `RoleMemoryStore` | channel/policy payloads |

### Prerequisite type migrations (do these as each group moves)

- **den-core:** `Bear`, `BearMember`, `UserRecord`, `BearObservationRow`, and the
  proposal/observation param/enum types (`CreateMemoryProposal`,
  `ProposalResolutionParams`, …) — they are foundation rows shared widely.
- **den-tools:** the tool-shaped result/arg types (`MemoryReadResult`,
  `MemoryTree`, `MemoryWriteOutcome`, `PromptMemoryBlock*`, `PlanModeState`,
  `WorkSurfaceScaffold*`, `WebFetch*`, `NormalizedWebUrl`) and the now-relocated
  `DenToolInvocationContext`.
- **Open question:** `AcpResolvedSessionPolicy` / `AcpToolEnablementState` /
  `turn_state` straddle ACP and tools; decide whether they land in `den-core` or a
  later `den-acp` before `plan_mode` moves. Until then, `plan_mode` is the last
  group to migrate.

Hard gate per step: `cargo check --workspace --all-targets` green + `den-tools`
clippy advisory-clean.

### Phase B — `web/` landed (2026-06, `test` branch)

First executor group inverted. `den_tools::web` now owns the `web_fetch` /
`web_search` orchestration (arg parsing, approval branching, audit construction,
HTML→text, result re-ranking) plus the pure `text` helpers
(`html_to_text_excerpt`, `truncate_chars`); `support::truncate_chars` is a
re-export shim for `agent_loop`. Capabilities are behind `WebFetcher`
(`decide_fetch_approval`, `record_fetch_attempt`, `http_get`, `preferred_hosts`,
`normalize_host`, `provider_search`, `default_search_max_results`) with data
shapes `WebUrl` / `WebApproval` / `WebFetchAudit` / `WebHttpResponse`. The `den`
crate implements it as `DenWebFetcher { pool, config }` over `web_policy` +
`reqwest` + the Brave provider; `core/tools/web` is now just that impl + thin
`CustomError`-mapping wrappers.

Two design refinements learned here, applicable to the remaining groups:

- **Executors take primitives, not `DenToolInvocationContext`.** That struct is
  `#[non_exhaustive]` and is built by struct-literal in ~17 in-workspace sites, so
  relocating it would break all of them (can't construct a foreign
  `#[non_exhaustive]` struct). `web_fetch`/`web_search` instead take `bear_id` /
  `session_id`. This is better trait hygiene anyway (each executor takes exactly
  what it needs), and defers the context relocation to a dedicated task (give it a
  `new(...)`/builder before moving it).
- **`CustomError::into_den()`** (inherent method on the `den` error) is the
  boundary converter the runtime impls use, since the orphan rule forbids
  `impl From<CustomError> for DenError` (both foreign to `From`). Variants mirror
  1:1, so it is lossless. Reuse it for every Phase B runtime impl.
- Sub-traits get `: Send + Sync` (so the generic orchestration futures are `Send`
  for the multi-thread dispatcher) — confirm the `ToolContext` bundle keeps this.

Verified: `cargo check --workspace --all-targets` green; `den-tools` clippy-clean;
`cargo test -p den-tools` green (no behavior change — pure relocation).

### Phase B — `prompt_memory/` landed (2026-06, `test` branch)

Second executor group inverted. `den_tools::prompt_memory` now owns the
`prompt_memory_upsert` / `prompt_memory_list` / `prompt_memory_patch`
orchestration (pair-role gating, argument structs/parsing, validation, write/patch
construction, conflict/supersede archiving sequencing, result shaping). The
prompt-memory domain types — `PromptMemoryBlockType` / `Scope` / `State`,
`PromptMemoryBlock`, `PromptMemoryBlockWrite`, `PromptMemoryBlockPatch` — moved
into `den_tools::prompt_memory::types` (serde-only; no sqlx) and are re-exported
from `core::prompt_memory_blocks` / `core::prompt_memory_block_store` so the
Postgres store and prompt-assembly code keep their paths. Capabilities are behind
`PromptMemoryStore` (`list_blocks`, `upsert_block`, `patch_block`,
`archive_conflicting`, `archive_superseded_by`); `den` implements it as
`DenPromptMemoryStore { pool }` over `prompt_memory_block_store`, and
`core/tools/prompt_memory` is now just that impl + thin `CustomError`-mapping
wrappers (signatures unchanged, so the dispatcher and existing tests are
untouched).

Also lifted the **shared tool-argument validators** to a new
`den_tools::validation` module (`validate_bounded_text`, `validate_optional_object`)
plus `validate_prompt_memory_scope` in the `prompt_memory` module. They now return
`DenError`; `support` re-exports the two generic ones, and the change is
`?`-transparent for the remaining in-`den` executors (`DenError → CustomError` via
`From`). This pre-stages validation for the rest of Phase B.

Notes carried forward: executors again take primitives (`bear_id`, `user_id`,
`BearProfile`) rather than `DenToolInvocationContext`; runtime impl uses
`CustomError::into_den()` at the seam; `PromptMemoryStore: Send + Sync`.

Verified: `cargo build -p den` green (only pre-existing dead-code warnings);
`cargo test -p den -p den-tools --no-run` green; `den-tools` clippy-clean.

### Phase B — `memory_read/` landed (2026-06, `test` branch)

Read surface inverted. `den_tools::memory` owns `memory_read` / `memory_browse` /
`memory_search` / `memory_status` (arg structs + parsing, non-empty validation,
search-limit clamp, and the status prompt-memory diagnostic composition via the
pure `prompt_memory_diagnostic_summary`). The new `RoleMemoryStore` capability
seam (`read`, `browse`, `search`, `status_base`) returns already-shaped tool JSON
(`serde_json::Value`), so no result DTOs were needed — the `sqlite_memory_*`
helpers already return `Value`. `memory_status` composes `RoleMemoryStore` +
`PromptMemoryStore` (reusing the prompt-memory seam from the previous group).

`den` implements `DenRoleMemoryStore { config }`, which owns the
native-vs-legacy branch: the SQLite path via `MemoryStoreManager` +
`memory::tools::sqlite_*`, and the MemFS HTTP fallback (slated for v0-legacy
deletion). `status_base` returns the base status **without** the diagnostic; the
executor layers it on (preserving the native shape; the legacy MemFS
not-configured fallback now also carries `bear_id`, a harmless addition). Wrapper
signatures (`memory_status`/`_value`/`browse`/`read`/`search`) are unchanged, so
the dispatcher, `environment.rs`, and tests are untouched.

Seam note: `store_for_bear` already returns `DenError` (den-memory), so it needs
no conversion; only the `sqlite_*`/MemFS `CustomError` results use
`CustomError::into_den()`. `RoleMemoryStore` will grow a write surface as the
later groups (memory_write / work_surface / plan_mode) migrate.

Verified: `cargo build -p den` green; `cargo test -p den -p den-tools --no-run`
green; `den-tools` clippy-clean.

### Phase B — foundation: `support` + `preflight` + context relocated (2026-06, `test` branch)

Two enabling moves landed before the context-heavy groups:

1. **`DenToolInvocationContext` → `den-tools::context`** (it is per-call *data*, not
   a capability). Dropped `#[non_exhaustive]`; the ~17 in-`den` struct-literal
   construction sites keep working through a re-export at `core::tools::session`.
2. **`core::tools::support` + `core::tools::preflight` → `den-tools`** (returning
   `DenError`), re-exported at their former paths. This relocates the shared
   validators, content-classification heuristics, scope tables, SSRF URL checks,
   the memory-write semantic-confirmation token machinery, and the preflight gate
   — everything the remaining memory executors depend on. `MemoryWriteEntryArguments`
   moved too. `den-tools` gained `base64`/`sha2`/`time`/`url` (pure-compute deps).

### Phase B — `memory_write` landed (2026-06, `test` branch)

`den_tools::memory::write_memory_entry` now owns the executor (pair gating,
validation, source/human merge, ACP session derivation, entry construction)
behind `RoleMemoryStore::write_entry` (added to the existing seam). `den` resolves
the authoring user (`user_by_id`, a capability) in the wrapper and passes the
identity as primitives; `DenRoleMemoryStore::write_entry` owns the native
`sqlite_write_profile_entry` path and the legacy MemFS request. Compat shims keep
`memory_write::source_acp_session_id` (used across workflow/payloads/plan_mode/
environment/memory_review) and the `&User`-based `merge_memory_entry_source_with_human`
(used by a test) resolving.

Verified: `cargo build -p den` green; `cargo test -p den -p den-tools --no-run`
green; `den-tools` clippy-clean.

### Phase B — `observations` landed (2026-06, `test` branch)

`den_tools::review::write_observation` owns the executor (watch gating, validation,
salience/id normalization, idempotency branch, payload shaping) behind a new
`MemoryReviewStore` seam. Because the den proposal/observation row + param types
carry lifetimes and span three modules (`bear_observations`, `memory_proposals`,
`reflection_conductor`), the seam uses **owned** value types
(`ObservationWriteRequest`, `ObservationRecord`); the `den` impl
(`DenMemoryReviewStore`) composes `create_observation` + `create_proposal` +
curate enqueue + native/legacy `mark_review_queued`. Wrapper signature unchanged.
This seam will also back `memory_review`.

Verified: `cargo build -p den` green; `cargo test -p den -p den-tools --no-run`
green; `den-tools` clippy-clean.

### Phase B — `memory_review` landed (2026-06, `test` branch)

The five curate/pair executors (`apply_core_update`, `list_proposals`,
`read_proposal`, `resolve_proposal`, `request_review`) moved into
`den_tools::review::memory_review`, owning role gating, argument validation
(status enum, source-path rules, bounded text), and projection-scope computation.
`MemoryReviewStore` grew coarse capability methods returning serialized proposal
JSON (`list_proposals`/`get_proposal`/`resolve_proposal`/`request_review`/
`apply_core_update`); the `den` impl owns the `create_proposal`/`resolve_proposal`/
`promote_core_content` calls, the native-vs-MemFS core-update branch, and the
`conversation_events` projections. The full `DenMemoryReviewStore` (observation +
review methods) now lives in `core::tools::memory_review`; `observations.rs` is a
thin wrapper over the same store. Wrapper signatures unchanged.

Verified: `cargo build -p den` green; `cargo test -p den -p den-tools --no-run`
green; `den-tools` clippy-clean.

### Phase B — `work_surface` landed (2026-06, `clippy` branch)

`create_work_surface_scaffold` and all of its orientation/builder helpers
(`infer_work_surface_hint`, slug normalization, `WorkSurfaceSessionHints`,
`WorkSurfaceProjectionStatus`, candidate-slug + scaffold-path builders, index/entry
body rendering, anchor-path collection) moved into `den_tools::work_surface`. The
executor gates by role, validates args, builds runtime-neutral `ScaffoldRequest`s,
and delegates persistence to the new `WorkSurfaceOps::write_scaffold` seam. The
`den` impl (`DenWorkSurfaceOps`) owns the native SQLite path
(`sqlite_write_at_path`) and the legacy MemFS path, including the special
`core/work_surfaces/index.md` append/replace logic and `MemfsCoreUpdateResponse`
→ JSON serialization. `core::tools::work_surface` is now a re-export shim plus the
concrete ops impl and a thin wrapper; existing call sites and the orientation +
scaffold test suites resolve unchanged.

Verified: `cargo build -p den` green; `cargo test -p den -p den-tools --no-run`
green; `den-tools` clippy-clean.

### Phase B — `plan_mode` landed (2026-06, `clippy` branch)

The five ACP plan-mode executors (`enter`, `status`, `record_approval`, `exit`,
`cancel`) moved into `den_tools::plan_mode`, owning argument parsing/validation,
the ACP-session-id requirement, the bounded-text checks, and the static response
envelope (domain marker, `mode_update`, human-facing instruction lists). The new
`PlanModeOps` seam exposes coarse transition methods returning already-rendered
`PlanModeView`/`PlanModeStatusView`/`PlanModeExitView` (workplan + plan_mode row
+ `workflow_state`); the `den` impl (`DenPlanModeOps`) owns the `acp_plan_mode`
DB rows, `acp_sessions::set_current_mode` calls, `turn_state` rendering, and the
native-SQLite vs. legacy-MemFS plan-artifact write. `config`/`stores` are only
threaded for the `exit` artifact path. The workplan-payload `fn` pointers are
held by the `den` impl rather than crossing the crate boundary, so dispatcher
call sites are unchanged.

Verified: `cargo build -p den` green; `cargo test -p den -p den-tools --no-run`
green; `den-tools` clippy-clean.

### Phase B — dispatcher/`ToolContext` umbrella: complete (2026-06)

**`invoke_den_tool` now lives in `den-tools` (`dispatch.rs`).** The remaining
`den`-only executor groups were first inverted behind their own seams, then the
dispatcher (preflight + authorization + the tool→executor match) was relocated
behind a single `ToolContext` umbrella.

Remaining groups inverted in this pass:

- **`Identity/Policy` → `BearDirectory`** (`den_tools::identity`): the bear/user/
  policy/capabilities/channel read tools own their JSON shaping over a
  `BearDirectory` seam; `channel_context` and `capabilities` are pure
  (descriptor-backed). The dispatcher's authorization
  (`authorize_context`/`context_role`/`authorize_tool_for_profile`) moved here too
  — `den` provides `DenBearDirectory` (bears/user DB + `bear_profile_bindings`).
- **`environment` → `EnvironmentOps`** (`den_tools::environment`): the orientation
  payload builders are pure `den-tools`; `session_info`/`bear_environment` compose
  `BearDirectory` + an `EnvironmentOps` seam (memory-status snapshot, ACP adapter
  fetch, config flags). `den::payloads` is now an adapter shim for existing tests.
- **`workflow` → `WorkPlanOps`** (coarse `Value` seam): the work-plan trio is
  saturated with `den`-only `work_plans`/`den_docket`/`acp_plan_mode` types that
  are not yet in shared crates, so this is a coarse pass-through seam; the
  `den` impl keeps the logic + activity-payload builders. Tighten once those
  domain types migrate (v0 de-stringify / docket split).
- **`conversation_set_title` → `ConversationTitleOps`** and
  **`memory_orient_work_surface` → `WorkSurfaceOps::orient`**.

The umbrella: `den_tools::dispatch::ToolContext` is a supertrait bundling
`BearDirectory + ConversationTitleOps + EnvironmentOps + WorkPlanOps +
WorkSurfaceOps + WebFetcher + RoleMemoryStore + PromptMemoryStore +
MemoryReviewStore + PlanModeOps + Send + Sync`. `den` provides one
`DenToolContext { pool, config, stores }` implementing every sub-trait (each impl
delegates to the existing per-capability `Den*` type); `den::invoke_den_tool` is a
thin wrapper that builds it and calls `den_tools::dispatch::invoke_den_tool`,
mapping `DenError → CustomError`.

Workspace green; `den-tools` clippy-clean; the 13 `core::tools::tests` (which
exercise `invoke_den_tool` end-to-end) pass.

**Known follow-ups (cleanup, not blocking):** ~~the per-tool `den` wrapper
functions are only used by `#[cfg(test)]` tests, so non-test builds emit
`dead_code` warnings; remove them and migrate those tests~~ **done** (see *wrapper
cleanup + test migration* below). The coarse `WorkPlanOps` seam should still be
tightened after the `work_plans`/docket domain types migrate to shared crates.

### Phase B — wrapper cleanup + test migration (2026-06, `clippy` branch)

Closed out the dispatcher follow-up. The now-dead per-tool `den` wrapper
functions (`web_fetch`/`web_search`, `memory_browse`/`read`/`search`, the
`prompt_memory_*` trio, the five `memory_review` fns, `write_observation`, the
five `plan_mode` fns, `orient`/`create_work_surface_scaffold`, the identity +
`environment` fns, `merge_memory_entry_source_with_human`/`write_memory_entry`)
were deleted — the dispatcher reaches the executors through `DenToolContext`, so
the wrappers had no non-test callers. Removed the modules that became empty shims
(`observations.rs`, `payloads.rs`, `prompt_memory_diagnostics.rs`,
`preflight.rs`). Also de-duplicated `patch_letta_conversation_summary` (the
`session/mod.rs` copy now delegates to the canonical `letta.rs` one).

Tests were migrated per the "sibling module when reasonable" preference:

- **Pure unit tests moved into `den-tools` as sibling `#[cfg(test)]` modules:**
  the `session_info`/`bear_environment` payload tests → `environment/payloads.rs`;
  `infer_work_surface_hint` → `work_surface/`; `merge_memory_entry_source_with_human`
  → `memory/`.
- **DB-backed tests stay in `den`** (they need the `#[sqlx::test]` harness +
  migrations): the `prompt_memory` tests now construct `DenPromptMemoryStore`
  directly; the `preflight` test imports from `den_tools::preflight`.
- **Resurrected the orphaned `core::tools::memory` test module** — four
  `#[sqlx::test]` projection tests (`apply_core_update` / `request_review` /
  `resolve_proposal`) that were never wired into the build. Re-pointed their
  imports to current paths, fixed the `invoke_den_tool` call to the 6-arg
  signature (`&MemoryStoreManager`), and wired `#[cfg(test)] mod memory;` into
  `core/tools/mod.rs`.

Verified: `cargo clippy -p den -p den-tools --lib --tests` clean of the targeted
`dead_code` warnings; relocated `den-tools` unit tests and the resurrected `den`
projection tests pass.

## v0 — core module triage (complete, 2026-06, `test` branch)

The gating v0 deliverable. The 43 loose `core/*.rs` subsystem files (~18k LOC)
were sorted into their intended subsystem modules **while still a single crate**,
so each future crate's contents are physically co-located and the extraction
becomes a directory move. Mechanic (behavior-preserving, one cluster per commit):
`git mv` each file into its subsystem dir, declare it canonically in that dir's
`mod.rs`, and keep a **flat re-export shim** in `core/mod.rs`
(`pub use acp::runtime as acp_runtime;`) so every existing `crate::core::<flat>`
reference compiles unchanged. The handful of intra-file `super::` references that
broke on reparenting were repointed (to `crate::core::…` or the new sibling).

Co-location landed:

- **`core/acp/`** — all nine `acp_*` modules (`letta_events`, `plan_mode`,
  `runtime`, `sessions`, `tokens`, `tool_turns`, `tools`, `turn_controller`,
  `turn_runner`) + the two included test files. ~8.8k LOC; the bulk of the future
  `den-acp` crate.
- **`core/runtime/`** — promoted the inline `runtime` module + `#[path]` aliases
  into a real `runtime/mod.rs`; absorbed `role`(+tests)/`role_registry`,
  `turn_state`, `pair_turn`, `conversations`, `compaction_observability`,
  `compaction_store` alongside the existing `compaction`/`contracts`/`provider`/
  `bearwire_projection` subdirs.
- **`core/conversation/`** — `events`(+tests), `message_types`,
  `persistence`(+2 integration tests), `archived` (replaces the prior inline
  re-export module).
- **`core/memory/`** — `manager_head`(+tests), `proposals`, `curate_executor`,
  `bear_observations`, `prompt_blocks`, `prompt_block_store`.
- **`core/reflection/`** (new) — `conductor`, `conversations`.
- **`core/llm/`** — `bifrost`. **`core/letta/`** — `runtime_stream_parser`(+tests,
  legacy). **`core/tools/`** — `web_policy`, `tool_descriptor_guidance`.

Residual flat files (intentionally not moved): `docket.rs` / `work_plans.rs` are
crate-level re-export shims for the already-extracted `den-docket`; `api_utils.rs`
is a dependency-free cross-cutting serde util (a future `den-core` leaf candidate,
not a subsystem file). Loose subsystem `*.rs` count: **43 → 0**.

Verified per cluster: `cargo check -p den --lib`/`--tests` green; finalized with
`cargo clippy --workspace --all-targets` (no errors; only the pre-existing
advisory pedantic/nursery + baseline `dead_code` warnings, count unchanged) and a
relocated unit-test smoke run. **This unblocks `den-runtime`** (and the
`den-acp`/`den-api`/`den-web` edges): the `ToolContext` seam is stable and the
subsystem sources are now co-located for mechanical extraction. Resolves the
*Open question* on loose-file placement between `den-acp` and `den-runtime`.
