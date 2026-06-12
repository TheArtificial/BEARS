# Den Crate Split and Rust Idiom Refactor Plan

> **Status (2026-06): draft for discussion.** This plan extracts the crate-boundary ("Option B") portion of [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md) into its own roadmap item and broadens it. Docket's own work (the `core/docket/` module and `DocketService` trait seam) stays in that plan. This document covers (1) turning the single `den` crate into a Cargo workspace and (2) using that effort as a thorough refactor toward idiomatic Rust — clippy-driven, with "stringy" structured arguments replaced by proper types. Canonical runtime context: [Den-Native Runtime](../architecture/den-native-runtime.md).
>
> **Status update (2026-06):** v0/v1 groundwork has begun on the `clippy` branch — workspace + lint table are in, `den-core` is seeded (`config`, `metrics`), and a re-export extraction technique is validated. A blocker (the web-coupled shared error type) gates the service-crate extractions. See *Execution log* at the end.
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

Layered DAG, leaves at the bottom; both former "big crates" are split. Sizes are rough current LOC; "new" does not exist yet. Exact contents of `den-runtime` / `den-acp` / `den-tools` are finalized by the v0 triage.

```
                              den (bin: startup, wiring, anyhow)        ~0.5k
                  /        |          |            |        \
                 v         v          v            v         v
            den-web    den-api    den-acp     (workspace tests)        ~13.5k / ~10-12k / ~10-15k
                  \        |          /
                   \       |         /
                    v      v        v
                        den-runtime  (loop, governance, harnesses)      ~12-18k
                    /     |      |       \
                   v      v      v        v
              den-llm  den-tools  (impl ToolContext / TaskDispatcher)
                   \      |   \      /
                    \     |    v    v
                     \    |  den-memory   den-docket                     ~2.2k / ~1-3k(new)
                      \   |   /              /
                       v  v  v              v
                            den-core  (types, errors, config, pool,      ~3-5k (leaf)
                                       migrations, observability)
```

| Crate | Role | Depends on | Source today | Churn |
|-------|------|------------|--------------|-------|
| **`den-core`** | Foundation leaf: shared domain **newtypes** (bear/job/task/run/session ids) and **enums** (role, trust profile, governance mode, statuses), error types, config, `PgPool`/`SqlitePool` handles, migrations runner, tracing/observability setup. Third-party deps only. | `errors/`, `observability/`, `config.rs`, shared model types | low |
| **`den-llm`** | Bifrost/LLM gateway client + streaming. Stable, widely depended on. | `den-core` | low |
| **`den-memory`** | Bear memory: `MemoryStore` trait + per-Bear SQLite impl + curation. **Native today** (see *Memory status*). | `den-core` | medium |
| **`den-docket`** | Jobs/tasks orchestration (Postgres): `DocketService` + `TaskDispatcher` traits + impl. Never executes task bodies. | `den-core` | medium |
| **`den-tools`** | Model-facing tool surface: descriptors, registry, executors; defines `ToolContext`. Descriptor-owned names (no scattered alias `match` arms). | `den-core`, `den-memory`, `den-docket`, `den-llm` | high |
| **`den-runtime`** | Native agent loop, governance/supervision modes, context assembly, role harnesses, reflection. Implements `ToolContext` and `TaskDispatcher`. | `den-core`, `den-llm`, `den-memory`, `den-docket`, `den-tools` | high |
| **`den-acp`** | ACP protocol runtime + its HTTP surface: turn runner, sessions, adapter event mapping, the loose `core/acp_*.rs`, `api/acp/`. | `den-runtime`, `den-core` | high |
| **`den-api`** | Non-ACP HTTP/JSON API (`/v1` chat, bears, admin JSON) + OpenAPI. | `den-runtime` (+ service traits) | high |
| **`den-web`** | Server-rendered admin UI + minijinja templates + template embedding (`build.rs`). | `den-runtime` (+ service traits) | high |
| **`den`** | Binary: `main`, startup, dependency injection (constructs concrete services, injects dispatcher + tool context), config load, router composition (mounts api/acp/web). | everything | low |

**Build-time payoff:** `den-web` extraction stops HTML edits from rebuilding logic; `den-acp` and `den-api` isolate the two highest-churn HTTP surfaces from each other and from runtime; `den-tools` isolates the ~8.7k tool surface; `den-core`/`den-llm` are stable leaves most edits never touch; service crates behind stable traits don't cascade. Tests become per-crate; crates compile in parallel.

**Deliberately not crates:** legacy `core/letta/` (~3.9k) and `core/codepool/` (~0.5k) are slated for deletion in the native migration ([DEN_NATIVE_RUNTIME_PLAN.md](DEN_NATIVE_RUNTIME_PLAN.md)); do not crateify them — delete in/around v0.

## Crate naming (decided: `den-core` + binary `den`)

The foundation leaf is **`den-core`** and the **binary stays `den`**. Rationale (recorded for posterity):

- **`*-core` is the dominant Rust idiom** for a foundational leaf everything depends on: `tracing-core`, `futures-core`, `serde_core`, `regex-syntax`/`regex-automata`, the `tower` ecosystem. `-foundation` reads as Swift/Apple, not Rust.
- **Naming the leaf `den` would force a binary rename** (two crates can't share a name), changing the build artifact and every Dockerfile/deploy reference for no benefit. Keeping `den` as the binary preserves "den = the product you run."

Optional later refinement (deferred): split `den-core` into `den-core` (pure types/errors) + `den-db` (pool/migrations) so type-only crates don't pull `sqlx`. Not in v1.

## Rust idiom refactor (in-scope)

The split is also the vehicle for paying down idiom debt. Two workstreams run alongside the extraction.

### clippy as the advisor and gate

- Adopt a workspace lint table (`[workspace.lints.clippy]` in the root `Cargo.toml`, Cargo ≥1.74) so lint levels are single-sourced across all crates.
- v0 establishes a **clean baseline**: run `cargo clippy --workspace --all-targets --all-features`, fix the existing warnings, then make CI enforce `-D warnings`.
- Enable `clippy::pedantic` and `clippy::nursery` as **advisory** groups, allow-listing the noisy lints; use them to drive idiom fixes (needless clones, `&str` vs `String`, manual `map`/`and_then`, `expect` discipline, etc.) rather than as hard gates initially.

### De-stringify: typed arguments over "stringy" structures

Replace untyped structured data with Rust types, applying "parse, don't validate" at boundaries:

- **Ids → newtypes.** `BearId(Uuid)`, `JobId`, `TaskId`, `RunId`, `ConversationId`, `SessionId` instead of bare `Uuid`/`String`. Prevents argument transposition and clarifies signatures.
- **Closed sets → enums.** Role, trust profile, governance mode (ADR-0039), task/run/criterion status (ADR-0034), tool scope/side-effect kinds. Several already exist as enums; ensure DB mapping uses `sqlx::Type`/derive, not string `match`, and that no boundary re-stringifies them.
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

1. **Core module triage.** Sort the 43 loose `core/*.rs` files into their intended subsystem modules (`memory`, `docket`, `runtime`, `acp`, `tools`, `llm`, plus `api`/`web`), so each future crate's contents are co-located. This is the gating deliverable.
2. **Trait seams in place.** `DocketService` + `TaskDispatcher` (Docket plan), `MemoryStore`, and `ToolContext` exist as the public faces of their modules; cross-module access goes through them.
3. **clippy baseline.** Workspace lint table; fix existing warnings; CI `-D warnings`.
4. **De-stringify the shared core.** Land the id newtypes and enums that will live in `den-core`, plus typed errors/config, so the foundation crate is already idiomatic when extracted.
5. **Delete legacy.** Remove `core/letta/` and `core/codepool/` per the native migration.

### v1 — Workspace split (extract + idiomatize, leaves→edges)

Each step keeps the workspace green and is behavior-free; idiom tightening for moved modules rides along.

1. Workspace skeleton + **`den-core`** (mechanical moves + `crate::` → `den_core::`).
2. **`den-llm`** (stable leaf).
3. **`den-memory`** (trait face ready now), **`den-docket`** (after Docket Phase 1 module lands), **`den-tools`**.
4. **`den-runtime`** (implements the inversion traits).
5. **`den-acp`**, then **`den-api`**, **`den-web`** (edges).
6. The original `den` crate collapses to the thin binary: startup, DI, router composition.

### v2 — Deferred refinements

`den-core`/`den-db` split, native `async fn` in traits (drop `async-trait`), pedantic/nursery promoted from advisory to gating once clean.

## Caveats

- **High-churn trait crates.** Crate boundaries help only across **stable** public traits; if `DocketService`/`MemoryStore`/`TaskDispatcher`/`ToolContext` signatures churn, dependents rebuild. Hence v0 stabilizes them first.
- **sqlx offline data per DB crate.** Compile-time-checked `query!`/`query_as!` need `DATABASE_URL` or committed `.sqlx` offline data; each DB-touching crate (`den-core`/db, `den-memory`, `den-docket`, query-issuing parts of `den-api`/`den-acp`) needs offline data and `SQLX_OFFLINE` wired in CI.
- **`async-trait` cost.** Service traits are async; `async-trait` boxes futures (negligible at our call rates). Removed in v2 via native async-fn-in-trait.
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
- [ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md): trust profile / governance mode enums are prime de-stringify targets in `den-core`.
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
- **`den-core` seeded.** Extracted two clean leaves into `crates/den-core`:
  `config` (only `url`/`tracing`/`dotenvy`/std) and `metrics` (std-only,
  formerly `observability::metrics`). `Config::test_stub` is exposed via a
  `den-core` `test-util` feature.
- **Default-level clippy machine-fixes** applied across core/api/web.

**Validated technique — re-export shim for low-churn extraction.** Moving a
module into a member crate and then `pub use`ing it back at the original crate
path (e.g. `pub use den_core::config;` in `den/src/lib.rs`, `pub use
den_core::metrics;` in `observability/mod.rs`) lets every existing `crate::…`
reference compile unchanged. This makes each extraction a small, reviewable
move rather than a repo-wide path rewrite. **Recommended as the standard
extraction mechanic** (replaces the "replace intra-crate `crate::` paths"
step in *Migration mechanics* for the common case).

**Blocker found — the shared error type gates service-crate extraction.**
`crate::errors::CustomError` is the de-facto shared error across nearly every
module, but it is **web-coupled**: it implements `axum::IntoResponse` by
rendering the `error.html` minijinja template (dev: `path_loader`; prod:
`minijinja_embed::load_templates!`, which embeds from the `den` crate's build
context), and it carries `From<auth_backend::…>` impls. It therefore cannot
move into the `den-core` leaf without dragging axum + minijinja + embedded
templates into the foundation, and the orphan rule forbids keeping the
`IntoResponse` impl in `den` once the type moves. Because `den-llm`,
`den-memory`, `den-docket`, and `den-tools` all return `CustomError` today,
**none of them can be extracted to depend only on `den-core` until this is
resolved.**

This makes error decoupling the **true v0 gate** for the service crates (ahead
of, or alongside, the module triage). Options, needing a decision:

1. **Per-crate `thiserror` errors (plan's stated direction).** Each subsystem
   defines its own typed error; the HTTP surfaces (`den-api`/`den-acp`/`den-web`)
   map those into responses. Most idiomatic; largest diff.
2. **`den-core` error + `den-web` response adapter.** Move a web-free
   `CoreError` enum (Display/`Error` + infra `From` impls) into `den-core`;
   keep `IntoResponse`/template rendering in a `den`/`den-web` layer (newtype or
   local wrapper to satisfy the orphan rule). Smaller diff; keeps one shared
   error.

Both are behavior-sensitive (error-page rendering), so this was intentionally
**not** done autonomously and is left for explicit direction.

**Suggested next steps:** (a) pick an error-decoupling option above; (b) then
extract `den-llm` (next clean-ish leaf once its error dependency is typed);
(c) proceed with module triage co-locating the loose `core/*.rs` files per the
v0 plan.
