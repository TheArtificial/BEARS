# Rust Clarity & Idiomatic-ness Audit

Status: COMPLETE — see "Audit complete" near the bottom.
Scope: `services/den/crates/*`, `services/den/src`, `tools/bear-armature`.

Legend: [ ] not fixed, [x] fixed, (file:line) pointer.

---

## Remediation progress (in-flight)

Working through fixes in batches, running `cargo check`/`clippy` and committing at
intervals. This section is the recoverable log of what has been changed.

### Batch 1 — panic-safety + clippy-gate green (upstream crates) — DONE
Key discovery: the repo clippy gate (`cargo clippy --workspace --all-targets -- -D
warnings`, see `scripts/lint.sh`) was **red** under clippy 1.96.1. Fixing it is
high-value and unblocks CI. Working through it crate-by-crate in dependency order.

Panic-safety (audit theme 5 + UTF-8 byte-slice bugs):
- [x] `den-runtime/gateway_events.rs` `preview_str_truncated` — char-boundary safe.
- [x] `den-runtime/native_runtime/tools.rs` `first_sentence[..96]` — char-boundary safe.
- [x] `den-runtime/agent_assist/conversation_title.rs` `truncate_at_word_boundary` — char-boundary safe.
- [x] `den-runtime/agent_loop/session_store.rs` — 4x `.lock().expect()` → poison-tolerant `unwrap_or_else(PoisonError::into_inner)`.

Idiomatic/clippy fixes:
- [x] `den-runtime/agent_loop/budget.rs` — hand-rolled `Default` → `#[derive(Default)]`.
- [x] `den-runtime/runtime/bearwire_projection/wire.rs` — hand-rolled `Default` → derive + `#[default]`.
- [x] `den-runtime/reflection/archive_harvest.rs` — redundant `let _ = …?` removed (2x).
- [x] `den-core/tools/environment/payloads.rs` — 2 redundant `.clone()` on `memory_scope`.
- [x] `den-core/client_tools.rs:264` — `.filter().is_none()` → `is_none_or`.
- [x] `den-core/tools/result_compaction/tests.rs` — `Some("".to_string())` → `Some(String::new())` (3x).
- [x] `den-docket/integration_tests.rs` — needless raw-string hashes (2x).
- [x] `den-docket/db.rs:1558` — `.filter().is_none()` → `is_none_or`.
- [x] `den-docket/model.rs` — `TaskListCheckoutSource::LocalProjection` boxed (large_enum_variant); match site in `service.rs` deref'd.
- [x] `den-llm/client.rs:160` — `push_str("…")` → `push('…')`.
- [x] `den-llm/client.rs` + `embeddings.rs` — `Duration::from_secs` → `from_mins`.
- [x] `den-llm/model_registry.rs` — `unwrap_or(fn call)` → `unwrap_or_else`; `iter().any()` → `contains`.
- [x] `den-memory/import.rs:480` — `iter().any()` → `contains`.

### Batch 2 — den-service clippy green — DONE
- [x] `den-service` — all lib + test clippy errors fixed (raw-string hashes, `is_none_or`,
  `unwrap_or_else`, `from_mins`, `String::new()`, struct field order, needless `Ok(?)`).
  Verified `cargo clippy -p den-service --all-targets -- -D warnings` exits 0.

### Batch 3 — den-runtime clippy green — DONE
- [x] `den-runtime` — 69 lib / 73 test errors. Bulk via `cargo clippy --fix` (raw-string
  hashes ×52, derive `Eq`, `redundant_clone`, `clone_from`), remainder by hand:
  `large_enum_variant` on `PermissionResultCoordinatorOutcome::DispatchLocalTool`
  (boxed `tool_obligation`; consumers auto-deref), `unwrap_or_else`/`or_else`,
  dead `if selected_paths.is_empty() {"available"} else {"available"}` (audit
  assembler.rs:113 finding), `let...else` → `?`. Verified green; workspace compiles.

### Batch 4 — web-layer crates clippy green — DONE
- [x] `den-http`, `den-oauth`, `den-api`, `den-web`, `den-bearwire` — all green
  (`cargo clippy -p … --all-targets -- -D warnings` exits 0). Autofix + manual:
  `clone_from`, `unwrap_or_else`, `from_secs`, needless-borrow, collapsible-if,
  `doc_lazy_continuation`, and a restructure of `available_model_matches`
  (bearwire run.rs) making the intentional id cross-product explicit (clippy
  `suspicious_operation_groupings` false positive). Also fixed a genuine
  pre-existing compile break in a den-web test helper (missing `content_json`
  field on `PersistedConversationMessage`).

### Batch 5 — root `den` package (bin + integration tests) — IN PROGRESS
Root package production code (lib + bins) is clippy-green. The remaining gate
failures are **pre-existing broken test targets** from recent refactors:
- commit "Remove re-exports" dropped `den_runtime`'s public re-exports, so tests
  importing `den_runtime::{bears,tool_turns,turn_controller,prompt_memory_block_store,
  prompt_memory_blocks,conversation_persistence,runtime_contracts,…}` no longer resolve.
- work-plan tools were removed, so `DEN_WORK_PLAN_*` constants are gone.
- one test file (`apply_core_update_projection.rs`) had been truncated to 8 lines.

Module remap table (test imports → canonical crate):
- `den_runtime::bears` → `den_service::bears`
- `den_runtime::tool_turns` → `den_service::tool_turns`
- `den_runtime::turn_controller` → `den_service::turn_controller`
- `den_runtime::prompt_memory_block_store` → `den_service::prompt_memory_block_store`
- `den_runtime::prompt_memory_blocks` → `den_service::prompt_memory_blocks`
- `den_runtime::conversation_persistence` → `den_service::conversation::persistence`
- `den_runtime::runtime_contracts::{Runtime*}` → `den_protocol::{…}`
- `den_runtime::den_memory` → `den_memory`
- `DEN_WORK_PLAN_*` → removed; delete the assertions/tests exercising them.

Batch 5 — DONE. The whole workspace gate is green:
`cargo clippy --workspace --all-targets -- -D warnings` exits 0, and
`cargo test -p den --no-run` compiles clean.
- [x] Restored full `apply_core_update_projection.rs` from git (pre-truncation),
  fixed its malformed use-block, remapped `bears`→`den_service`.
- [x] Deleted obsolete `tests/work_plans.rs` (its `WorkPlan*` docket types were removed).
- [x] `tests/acp_plan_mode.rs` — `acp_session_id` → `client_session_id`.
- [x] Applied the module remap table across ~15 root test files (bears/tool_turns/
  turn_controller/prompt_memory_*/conversation_persistence → den_service;
  runtime_contracts → den_protocol; den_memory).
- [x] Correction: the work-plan **tools** were **renamed** to `task_lists`, not removed —
  `DEN_WORK_PLAN_*` → `DEN_TASK_LISTS_*` (not deleted) in descriptor_aliases/role_scoping/session_info.
- [x] Added new `projected_memory`/`recalled_memory: None` fields to `DenToolInvocationContext`
  literals across 8 test files (struct gained fields upstream).
- [x] Deleted `src/core/conversation_persistence_non_acp_bridge_tests.rs` — imports-only,
  zero test fns (gutted by an earlier bad commit); removed its `mod` decl in `core/mod.rs`.
- [x] `tests/recall_indexer.rs` — `cloned_ref_to_slice_refs`: `&[body.clone()]` → `std::slice::from_ref(&body)`.

## Overall status of clippy-gate remediation
The strict clippy gate (`scripts/lint.sh`) was RED workspace-wide under clippy 1.96.1.
It is now GREEN across every crate and every target. Note: `#[sqlx::test]` DB-backed
tests are verified to COMPILE only — they were not executed (no database available).

---

## Crates covered so far
- [x] den-protocol (full)
- [x] den-api (full)
- [x] den-http (full)
- [x] den-llm (full)
- [x] den-docket (full)
- [x] den-oauth (full)
- [x] den-bearwire (full)
- [x] den-memory (full)
- [x] den-core (full)
- [x] den-runtime (complete — 62 of ~100 files, everything >50 lines covered)
- [x] den-service (most files)
- [x] den-web (most files)
- [x] services/den/src (top-level bin/lib) (full)
- [x] tools/bear-armature (mostly complete — all files reviewed except the `handle_request` dispatch match itself, already flagged as a god-function)

---

## Findings

### den-docket (services/den/crates/den-docket)
- [ ] `src/model.rs` (2290 lines) is a god-file mixing legacy WorkPlan types, Docket relational types, validation, and conversions with no submodule boundaries — split by concern.
- [ ] `src/db.rs` (2007 lines) is a god-file with all CRUD for jobs/tasks/criteria/execution/work-plans in one flat module — split by aggregate (jobs, tasks, work_plans, execution).
- [ ] `src/db.rs:845-1018` `execute_job` ~170-line function with 4 branches duplicating `update_job`/`DocketJobUpdate` construction — extract a "transition job to status X" helper.
- [ ] `src/db.rs:1090` `result_summary`/`blocked_reason` are both bare `Option<String>` on `DocketTaskRunStateUpdate` — introduce an enum (`TaskOutcomeNote::Summary`/`Blocked`) instead of overloading meaning.
- [ ] `src/db.rs:469-546` and `1315-1399` `update_job`/`update_task_definition` manually reimplement "use new or fall back to current" via repeated `.unwrap_or(&current.x)` — use a merge helper/builder.
- [ ] `src/model.rs:1592-1602` `parsed_owner_profile`/`parsed_visibility`/`parsed_status` are near-identical one-line wrapper methods — collapse to a single generic accessor or inline `.parse()` at call sites.
- [ ] `src/model.rs:1426-1431` `docket_parent_task_ref` recovers a Uuid via string search/parse over `Vec<String>` refs (`format!("docket_parent_task:{id}")`/`strip_prefix`) — fragile stringly-typed encoding, should be a typed field.
- [ ] `src/db.rs:792-815`, `model.rs:1418-1424` — `execution_session_id`/`docket_checkout_refs` build tagged unions via string prefixes (`"acp:"`, `"conversation:"`, `"docket_job:"`) instead of an enum.
- [ ] `src/model.rs:474-478`, `1276-1280` — two near-identical `impl From<XValidationError> for DenError` blocks just stringify via Display — could share a helper.
- [ ] `src/dispatcher.rs:61-64` — informal "ponytail" codename/rationale embedded as an inline comment in library code — belongs in a doc comment or linked ADR.
- [ ] `src/db.rs` — repeated `unwrap()`/`expect()` on DB results outside tests risk panics instead of propagating via `?`.
- [ ] `src/service.rs`, `dispatcher.rs` — errors frequently stringified (`format!`/`to_string`) rather than modeled via a `thiserror` enum, losing structured error info; dispatcher swallows/stringifies underlying errors instead of `?` + `From` impl.
- [ ] `src/db.rs` — many query functions clone owned `String`/`Vec` into SQL params where `&str`/`&[T]` would do.
- [ ] `src/model.rs` — several types hand-write constructors/Display-like formatting instead of deriving `Default`/`From`/`Display`.
- [ ] `src/integration_tests.rs` — long, deeply nested tests duplicate setup boilerplate; factor into shared helpers.
- [ ] `src/service.rs` — overly generic parameter types (raw strings/ids) without newtypes make call sites easy to misuse.

### den-oauth (services/den/crates/den-oauth)
- [ ] `src/oauth/mod.rs:171,448` — `parse_scopes`/`_scopes_to_json` (dead-code-marked but public) duplicate `scopes_from_json`/`utils::scopes_to_json` byte-for-byte in 2-3 places — consolidate.
- [ ] `src/oauth/endpoints.rs:412-607` — `token_post` ~200-line god function mixing logging/validation/client auth/PKCE/token gen/DB writes/response building — decompose into named steps.
- [ ] `src/oauth/endpoints.rs:744-842` — `validate_pkce` nests match-in-match up to 4 levels with tracing interleaved at every branch — flatten with early returns, move tracing to a wrapping layer.
- [ ] `src/oauth/endpoints.rs:1203-1229` vs `auth.rs:38-69` — two near-identical `extract_bearer_token` impls with subtly different validation (one trims/checks empty, other doesn't) — unify.
- [ ] `src/oauth/db.rs` — ~10 `SELECT` blocks manually map sqlx rows field-by-field instead of `sqlx::FromRow`/`query_as!`; each repeats the same nullable-timestamp fallback.
- [ ] `src/oauth/error.rs:203-218` — `From<CustomError> for OAuthError` silently collapses unrelated variants and discards original message on fallback — lossy trait boundary.
- [ ] `src/templates/mod.rs:137-151` — `generate_csrf_token` uses `DefaultHasher` timestamp hashing, doc comment admits it's a never-replaced placeholder, and it's unused for real validation.
- [x] `src/oauth/utils.rs:20-32` vs `jwt.rs:239-251` — `generate_secure_random_string` and `generate_jti` duplicate the same charset-sampling loop.
- [ ] `src/oauth/mod.rs:6-14` — crate-wide `#![allow(clippy::...)]` silences 9 lints (incl. `too_many_arguments`, `result_large_err`) instead of fixing signatures (e.g. `create_authorization_code` takes 9 positional args — should be a struct).
- [ ] `src/oauth/endpoints.rs:1281-1300` vs `error.rs:110-125` — `bearer_error_response`/`oauth_error_response` reimplement `OAuthError -> StatusCode` mapping instead of calling `OAuthError::status_code()` — mapping now lives in 3 places that can drift.

### services/den/src (top-level binary/lib)
- [ ] `main.rs:58` vs `reindex.rs`/`import_legacy_memory.rs` — inconsistent `--help` convention: `return Ok(())` vs `std::process::exit(0)`.
- [ ] `main.rs:22`, `reindex.rs`, `import_legacy_memory.rs` — three near-identical hand-rolled argv-parsing loops (~80 duplicated lines) — extract shared helper or adopt `clap`.
- [ ] `lib.rs:129` `run_server` ~270-line god function (tracing setup, config validation, DB connect/migrate, session store, 4 worker-spawn blocks) — split into `spawn_web`/`spawn_api`/`spawn_workers`.
- [ ] `lib.rs:294-372` — four nearly identical `task_set.spawn(...)` blocks differing only in worker fn/interval — loop over `Vec<(&str, WorkerFn, Duration)>` or macro.
- [ ] `lib.rs` (multiple lines) — 7+ repeated `config.clone()`/`sqlx_pool.clone()` before spawns; consider an `AppHandles { pool, config }: Clone` struct for readability.
- [ ] `internal_tools.rs:94-117` — `authorize_internal_request` returns `Result<(), Box<Response>>` just to do `if let Err(response) = ...`; use `Option<Response>` or restructure to avoid boxing.
- [ ] `internal_tools.rs:106` — `raw.to_str().unwrap_or_default()` silently treats non-UTF8 Authorization header as empty/unauthorized — add a comment, easy to misread as a bug.
- [ ] `seeds.rs:145` — `ensure_bear(pool, slug, _config)` has unused `_config` param — wire through or drop.
- [ ] `seeds.rs:116,118` — allocates owned `String`s from `&'static str` constants inconsistently with borrow-friendly style elsewhere.
- [ ] `startup.rs:11` — `StartupError::Message`/`Tracing`/`SessionStore` are stringly-typed catch-alls inside an otherwise well-structured `thiserror` enum — give truly distinct errors their own variants.
- [ ] `startup.rs:43-52` — `sqlx_migrate_ignore_missing_from_env` hand-rolls bool-env parsing instead of a shared helper/`.parse::<bool>()`.
- [ ] `reindex.rs:43` — `--help` calls `std::process::exit(0)` inside a function typed to return `anyhow::Result`, mixing process-exit side effects into a "pure" parser.
- [ ] `lib.rs:39-94` — `NativeWebChatRuntime` composition-root glue has no doc comment explaining the indirection, unlike other documented composition decisions in the same file.
- [ ] `lib.rs:398-431` — `tracing_filter: String` declared unassigned then conditionally set via `#[cfg]` blocks — reads as a borrow-checker workaround; prefer a single `if/else` expression assigned directly.
- Overall: generally clear, good module-level docs and `thiserror` usage in `startup.rs`; main issues are duplication (`run_server`, CLI arg loops) and small inconsistencies.

### den-protocol
- [ ] `src/lib.rs:387` `classify_runtime_error` string-matches lowercased `err.to_string()` against a dozen substrings instead of using structured `DenError` variants/codes.
- [ ] `src/lib.rs:416` `runtime_error_is_no_active_runs_cancel` duplicates the same lowercase-substring pattern; a message change in the emitting code silently breaks this with no compiler help.
- [ ] `src/lib.rs` — seven marker/health-check traits live flat in `lib.rs` with no submodule boundaries separating DTOs/event enums/trait contracts (crate doc says otherwise).
- [ ] `src/lib.rs:379` `ToolActuatorRegistry` is an empty marker trait with no doc comment.
- Overall: small and mostly clean DTOs; error classification is fragile string-matching; traits need submodules.

### den-api
- [ ] `src/v1/profile.rs:60-82` `get_profile` chains four sequential `match...Ok/Err` blocks instead of `?` + `From`/`map_err` into `CustomError`.
- [ ] `src/v1/profile.rs:81` discards the underlying DB error entirely (`Err(_) => ... "Database error"`), bypassing the crate's own `CustomError`.
- [ ] `src/v1/profile.rs:98-176` three free functions manually build `Response`/`Box<Response>` with duplicated JSON error shapes, reimplementing `CustomError`'s `IntoResponse`.
- [ ] `src/v1/user.rs:198-246` `login` nests 3-deep if-let with duplicated `Err(CustomError::Authentication(...))` in two branches — flatten.
- [ ] `src/v1/user.rs` — pervasive `.to_string()` for static messages where `&'static str`/`Cow` would do.
- [ ] `src/v1/user.rs:280-282,308-309` `request_password_reset`/`confirm_password_reset` are TODO stubs that unconditionally return success — misleading public API contract.
- [ ] `src/service.rs:37-52` `api_readiness` inlines a `map_err` closure mixing logging + error mapping — extract a named helper.
- [ ] `src/service.rs:166-194,229-301` `create_session_layer`/`create_api_cors_layer` duplicate logic across `#[cfg(feature = "production")]` branches.
- [ ] `src/service.rs:55-79` doc comment on `create_api_app` is stale (`config`/`peer_routers` params undocumented).
- Overall: handler code mixes manual `Response` construction with `CustomError` idiom inconsistently; deep nesting that `?` would flatten.

### den-http
- [ ] `src/errors.rs:27-41,69-296` `CustomError` hand-rolls `Display`/`Error`/`From` impls with `String` payloads per variant despite `thiserror` being a direct dependency — should use `#[derive(thiserror::Error)]`.
- [ ] `src/auth_backend.rs:15-27` `Error` here correctly uses `thiserror::Error` derive — inconsistent with `errors::CustomError` in the same crate.
- [ ] `src/auth_backend.rs:104-113` vs `115-122` — duplicated `SessionUser{...}` literal construction — extract `From<db::UserAuth> for SessionUser`.
- [ ] `src/user/email_settings.rs:44-121` `settings_by_id` ~80-line nested if-let mixing lookup/insert/logging — split into lookup + create-default helpers.
- [ ] `src/user/email_settings.rs:390-436` `set_admin_email_verified` hand-rolls UPDATE-then-INSERT upsert instead of `INSERT ... ON CONFLICT` (used elsewhere in crate, e.g. `armature_tokens.rs:227-273`).
- [x] `src/email/mod.rs:83-87` `mailgun_client()` calls `.expect(...)` — panics in reachable library code, should return `Result`. **DONE** (safety batch): `mailgun_client()` now returns `Result<&Mailgun, DenError>` and `send_email_template` propagates initialization failure as `DenError::Email`; `den-http` clippy green.
- [ ] `src/email/mod.rs:105-107` manual "split before first dot" for `type_name` with no comment explaining significance.
- [ ] `src/user/mod.rs:11-18,22,36,43` dead commented-out struct/derive code left inline — remove.
- [ ] `src/user/mod.rs:79-108` `user_by_username_opt` takes `username: String` by value instead of `&str`, inconsistent with rest of `db.rs`.
- [ ] `src/user/db.rs:171` and `mod.rs:51` — two different `User` structs and two near-duplicate `user_by_id` functions with different fetch semantics — unclear which is canonical.
- Overall: most inconsistent crate audited so far — has good `thiserror` usage in one place (`auth_backend`) but hand-rolled boilerplate in its most important error type, plus one reachable `.expect()` panic.

### den-llm
- [ ] `src/client.rs:710-869` vs `871-1012` — `chat_completions_stream`/`responses_stream` are near-identical ~150-line functions differing mainly in endpoint/`LlmApiStyle` — extract shared helper.
- [ ] `src/client.rs:761-869,898-1011` — retry-without-headers block duplicated near-verbatim in both stream functions (~50 lines) — extract `retry_without_session_headers(...)`.
- [ ] `src/client.rs` (many sites) — nearly every error path uses `DenError::System(format!(...))` for 7+ semantically distinct failure modes, discarding structured error info.
- [ ] `src/client.rs:157-163` `log_sample` manually truncates strings; fine but candidate for shared utility if pattern recurs.
- [ ] `src/client.rs:165-251` `chat_message_chain_diagnostic` ~85-line function combining 3 distinct diagnostic computations in one loop — split into named helpers.
- [ ] `src/client.rs:500-557` `bifrost_virtual_key_not_found_diagnostic`/`bifrost_key_selection_diagnostic` duplicate the same telemetry-field extraction chains; `LlmRequestTelemetry::field` (line 254) already exists for this but isn't reused.
- [ ] `src/client.rs:1017,1034` `chat_completions_byte_stream`/`responses_byte_stream` are identical wrappers differing only in which stream fn they call — unify with an `ApiStyle` param.
- [ ] `src/model_registry.rs:200-353` `registry_entries()` ~150-line function rebuilds a `Vec` of a static catalog on every call — use a `const`/`LazyLock` table instead.
- [ ] `src/model_registry.rs:93-101` `entry_for_handle` does a linear scan over a freshly-rebuilt Vec — a `LazyLock` `HashMap` would be clearer and cheaper.
- Overall: good telemetry/observability, but `chat_completions_stream`/`responses_stream` are almost fully duplicated and nearly all errors funnel into `DenError::System(String)`.

### den-runtime (services/den/crates/den-runtime) — partial, 12 files covered
- [ ] `recall/query.rs:552` `merge_search_results` — 3 near-identical dedupe/push loops (vector/keyword/graph), could be one generic helper.
- [ ] `recall/query.rs:324` `graph_expand_hits` — 75-line god function mixing expansion/scoring/sorting/rendering.
- [ ] `recall/query.rs:409` `hybrid_memory_search` — 80-line function doing temporal parse + 3 search legs + merge + filter, each with duplicated best-effort/warn boilerplate.
- [ ] `recall/query.rs:56,231,296` — `disabled_projection` "reason" strings are stringly-typed (`"qdrant_unset"` etc.) rather than an enum.
- [ ] `bears/db.rs:33,48,315-317,339-341` — repeated `Bear`/`BearWithMembership` SELECT column lists not factored into a shared const (unlike `managed_blocks.rs`'s `SELECT_COLUMNS`).
- [ ] `bears/db.rs:742-753` `resolve_model_from_values` — 3x duplicated `.map(str::trim).filter(|s| !s.is_empty())` chain, extract `non_blank()` helper.
- [ ] `bears/managed_blocks.rs:11-23,412-424` `ManagedBlockResolutionRow` is an 11-tuple destructured positionally — fragile against column-order drift, should be a named-field struct via `FromRow`.
- [ ] `bears/managed_blocks.rs:382-471` `resolve_managed_blocks_for_bear` god function — extract per-row resolution into a helper.
- [ ] `bears/managed_blocks.rs:163-171` `managed_space_block_key` hardcodes a 5-arm match duplicating `BearProfile::as_str()` — use `format!` instead.
- [ ] `bears/managed_blocks.rs:33-75` — 3 near-identical hand-rolled `as_str()` impls on serde-derived enums — use `Display` once or `strum`.
- [ ] `native_runtime/web_chat_loop.rs:424-545` `poll_next` — ~120-line god function, 4+ level nested match; split `LoopPhase::Streaming` arm out.
- [ ] `native_runtime/web_chat_loop.rs:555-556` — silently swallows malformed tool-call args into empty object with no surfaced error; rename/comment to make intentional.
- [ ] `native_runtime/web_chat_loop.rs:267-343` `evaluate_completed_tool_batch` — nested god-closure for budget-advisory dedup, extract `push_budget_advisory()`.
- [ ] `native_runtime/web_chat_loop.rs` (616 lines) — borderline god-file combining state machine + tool execution + persistence; split persistence into its own module.
- [ ] `plan_mode.rs:131-136` `clean_optional` reimplements `Option::filter`+trim verbosely.
- [ ] `plan_mode.rs:250-279` `get_for_session` builds two different SQL strings via if/else with conditional `.bind()` arity — error-prone, split into two named query fns.
- [ ] `plan_mode.rs:420-495` `approve_plan_mode`/`reject_plan_mode`/`cancel_plan_mode` duplicate the "fetch current, decide client_session_id" preamble verbatim.
- [ ] `plan_mode.rs:497-513` `close_with_state` runtime-guards invalid states via `matches!` — model as a "ClosedPlanModeState" enum to make invalid states unrepresentable.
- [ ] `plan_mode.rs:281-333` `enter_plan_mode` mixes `pool` and `&mut *tx` reads inside one transaction — confusing.
- [ ] `memory/curate_executor.rs:50-113` `CurateTriage` enum has 5 variants with identical field shapes — should be a struct with a `kind` discriminant instead.
- [ ] `memory/curate_executor.rs:256-264` manual JSON-Map tally counter reimplements `HashMap::entry().or_insert()`.
- [ ] `memory/curate_executor.rs:397-451` `apply_core_promotion` needless tuple-then-immediate-use round trip.
- [ ] `turn_obligations.rs:19-76` `TurnObligationKind`/`ExpectedResponderAction` are two enums with byte-identical variants/impls — merge or macro-generate both (~50 duplicated lines).
- [ ] `turn_obligations.rs:149-167` `row_to_obligation` manually maps 15 fields — should derive `sqlx::FromRow` like `plan_mode.rs`/`managed_blocks.rs` do. **DEFERRED to DB env** (structural batch A): a clean FromRow conversion here (and in the sharing `turn_waits.rs`) requires adding `responder_ref_id`/`turn_step_id` to query shapes that currently omit them + dropping the tolerant `try_get().ok()` fallback — a change to *what the queries return / how the struct is populated*, which needs a runtime/DB test to trust. Held back from the no-DB batch.
- [ ] `turn_obligations.rs` — 4 nearly-identical wrapper-pairs (`_for_step` delegation) suggest an options-struct would flatten the API.
- [ ] `agent_loop/step.rs:203-293` `handshake_future` — 90-line god function, 4+ level nesting; extract retry/overflow-recovery branch.
- [ ] `agent_loop/step.rs:322-438` `recover_from_overflow_and_retry` duplicates `connect_request_stream`'s dispatch-by-`api_style` logic instead of reusing it — risk of drift.
- [ ] `agent_loop/step.rs:56,109,178,203` — associated functions take 7-8 positional params; use an options struct.
- [ ] `agent_assist/conversation_title.rs:262-273` `truncate_at_word_boundary` slices by raw byte index with no char-boundary safety/doc note (non-ASCII risk).
- [ ] `agent_assist/conversation_title.rs:275-304` two separate truncate-with-word-boundary implementations in the same file — consolidate.
- [ ] `agent_assist/conversation_title.rs:316-327` `is_uuid_like` hand-rolls UUID shape sniffing instead of `Uuid::try_parse`.
- [ ] `agent_assist/conversation_title.rs:329-365` `looks_like_machine_or_opaque_title` — 37-line function with 5 heuristics, split into named sub-checks.
- [ ] `client_obligation_coordinator.rs:88-162` `existing_tool_result_or_late`/`existing_permission_result_or_late` structurally identical except outcome type — share via generic helper.
- [ ] `client_obligation_coordinator.rs:412-482` `settle_permission_result` god function — extract `dispatch_local_tool_after_grant(...)`.
- [ ] `client_obligation_coordinator.rs:450-459` raw `Value` navigation for tool_name/arguments instead of a typed payload struct.
- [x] `agent_loop/key_memory_projection.rs:185-469` `project_key_memory` — ~285-line god function, 5 near-identical tier blocks — strongest extract-helper candidate in the crate. **DONE** (structural batch 6): extracted a `ProjectionTallies` bundle + an async `admit_record` helper encapsulating the shared budget→access→`try_take` admission; the five tiers now read as fetch→admit→assemble. Behavior preserved exactly (Tiers 1–2 keep the pre-fetch budget check; Tier 3 still silently skips unmapped records). Verified compile + `clippy --all-targets -D warnings` clean.
- [ ] `agent_loop/key_memory_projection.rs:63-158` `BudgetTracker::new` takes magic `tier_index: usize` (0-3) instead of an enum; every call site hardcodes a bare integer.
- [ ] `agent_loop/key_memory_projection.rs:132-157` magic number `3` for ellipsis length, plus inconsistent ellipsis style (`"..."` vs `"…"` in `recall/query.rs`) across files.
- [ ] `agent_loop/assembler.rs:220-460` `assemble_native_turn_for_bear` — 240-line god function, biggest split candidate (natural seams already exist per `system_text.push_str` block).
- [x] `agent_loop/assembler.rs:409-453` `Pair`/`Chat` branches are byte-for-byte duplicates — merge via `matches!(ctx.profile, BearProfile::Pair | BearProfile::Chat)`. **DONE** (structural batch 6): merged the two identical native-runtime fallback-pruning arms into one `matches!` guard. (Note: the `assembler.rs:113-121` dead ternary was already fixed in the clippy batch.)
- [ ] `agent_loop/assembler.rs:113-121` dead/always-true ternary (`if selected_paths.is_empty() {"available"} else {"available"}`) — leftover copy-paste.
- [ ] `agent_loop/assembler.rs:32-52` `AssembleTurnContext` has 16 public fields with semantic overlap — group into sub-structs.
- [ ] `agent_loop/assembler.rs:322-323` `.expect("session_id checked above")` — invariant enforced across two separate functions, fragile; thread the checked `Option` explicitly instead.
- Overall: `key_memory_projection.rs` and `assembler.rs` are the clearest split candidates (one oversized function each); `turn_obligations.rs` + `client_obligation_coordinator.rs` share a systemic parallel-enum/parallel-wrapper duplication pattern worth fixing together.
- **Not yet covered in den-runtime** (33.7k lines total, only ~12 files reviewed): remaining files under `agent_loop/`, `native_runtime/`, `recall/`, `bears/`, and any other top-level modules not listed above.

### den-memory (services/den/crates/den-memory)
- [ ] crate-wide — nearly every fallible call wraps as `DenError::System(format!("... failed: {e}"))`, discarding original error type; use a `thiserror` enum with `#[from]`/`#[source]`.
- [x] `src/reflection_outcomes.rs:70` `reflection_outcome_exists` swallows DB error via `.unwrap_or(0)`, treating real failures as "false". **DONE** (safety batch): function now returns `Result<bool, DenError>` and callers propagate DB failures; `den-memory`/`den-runtime` clippy green.
- [ ] `src/import.rs:181` `Err(_) => { branch_skipped += 1; continue; }` on `String::from_utf8` discards the actual UTF-8 error.
- [ ] crate-wide — `store.bear_id().to_string()` called on 60+ bind sites; cache a `bear_id_str()` or bind helper.
- [ ] `src/migrate.rs:12` `let names = columns;` needless rebinding.
- [ ] `src/import.rs:239-253` `ImportDraft` clones fields per commit unnecessarily inside a loop that already owns fresh values.
- [ ] `src/entity.rs:93,240` and similar constructors — repeated "row-from-just-written-args" template across `create_entity`/`attach_handle`/`append_relation`/`append_memory_promotion` — extract shared helper.
- [ ] `src/import.rs:118-305` `import_legacy_memory_source_inner` — ~190-line function mixing branch iteration/path filtering/git shelling/commit walking/frontmatter parsing/dry-run — decompose into per-commit/per-branch helpers.
- [ ] `src/import.rs:549-581` hand-rolled git subprocess wrapper duplicates `git2`-crate functionality.
- [ ] `src/promotions.rs:42-72` `promote_to_shared_core` returns unlabeled `Result<(String, String), DenError>` tuple — use a named struct.
- [ ] `src/tools.rs` (whole file) — every public fn returns untyped `serde_json::Value` instead of typed response structs, unlike rest of crate.
- [ ] `src/relations.rs:210` `descriptive_entity_ids_by_source` is dead public API — never re-exported/called outside its own test.
- [ ] `src/tools.rs:167-214` `sqlite_memory_search` builds `LIKE` pattern without escaping metacharacters, inconsistent with `admin_inspect::escape_like` (admin_inspect.rs:454-460) which does — reuse that helper.
- [ ] `src/descriptors.rs`, `logical_path.rs`, `entity.rs` — hand-written `as_str`/`parse` pairs for `RelationClass`/`EntityTrust`/`ResolutionState`/`MemoryScopeType` instead of `Display`/`FromStr`.
- [ ] 6+ files (`entity.rs`, `harvest.rs`, `promotions.rs`, `proposals.rs`, `reflection_outcomes.rs`, `records.rs`) — RFC3339 "now" formatting boilerplate duplicated verbatim; a `now_rfc3339()` helper already exists in `entity.rs`/`relations.rs` but isn't shared.
- [ ] `src/records.rs:763-781` `impl AsOfSqlRow` placed after a `#[cfg(test)]` block rather than near its struct def (line 561) — breaks file's ordering convention.
- [ ] `src/proposals.rs` — uses raw positional tuples as query row types instead of a named `FromRow` struct like sibling files — `row.0`/`row.1` indexing is hard to follow.
- [ ] `src/entity.rs:50`, `records.rs:82` — `#[allow(clippy::too_many_arguments)]` escape hatch instead of grouping params into an options struct.
- [ ] `src/import.rs:432` `&commit[..commit.len().min(12)]` byte-slices a String assuming ASCII hex SHA — undocumented assumption.
- [ ] `src/logical_path.rs:77-117` `from_logical_path` duplicates work_surfaces extraction logic twice (shared vs profile-local paths) — factor into one helper.
- Overall: disciplined and well-documented architecturally, but leans hard on stringly-typed errors and duplicates small helpers dozens of times instead of centralizing; `import.rs`/`tools.rs` most in need of decomposition/typing.

### den-core (services/den/crates/den-core)
- Error handling is clean overall: `DenError` is a well-designed thiserror-style enum, no stray unwrap/panic in library logic (the one `expect()` at `config.rs:508` for `DATABASE_URL` is legitimate startup-fatal config).
- [ ] `src/tools/descriptor/mod.rs` (1865 lines) — god-file: tool registry split across 4 independently-maintained match statements keyed on the same tool name (`builtin_den_tool_descriptors`, `den_tool_display`, `tool_domain`, `tool_content_class`) — drift risk when adding tools.
- [ ] `src/client_tools.rs` (1910 lines) — second god-file: dozens of near-identical `ToolPolicy`/`ClientToolDescriptor` const literals — candidate for builder/macro/table-driven approach.
- [ ] `src/tools/descriptor/mod.rs:1546` — `#[cfg(test)] mod tests` embedded mid-file, splitting schema-builder functions — move to end/own module.
- [ ] `src/tools/work_surface/mod.rs:229-246` `work_surface_scaffold_paths` returns unnamed 5-tuple destructured positionally at both call sites — use a named `WorkSurfacePaths` struct.
- [ ] `src/tools/web/mod.rs:47-89` `web_fetch` — "blocked" and "not approved" branches duplicate identical 12-line `record_fetch_attempt` call — factor into one audit-then-return helper.
- [ ] `src/tools/support.rs` (whole file) — mixes SSRF/URL validation, string cleaners, memory-content heuristic classifiers, and HMAC-ish confirmation-token plumbing under one "misc support" umbrella — split into separate modules.
- [ ] `src/tools/support.rs:33-134,136-179` `validate_memory_write_entry_semantics`/`assess_unlabeled_memory_misuse` duplicate the same classification cascade with different error framing — unify into one shared classifier returning an enum.
- [ ] `src/tools/prompt_memory/mod.rs:121-139,231-244` — fields cloned once into JSON-adjacent locals and again into the write/patch struct — restructure to avoid double clone.
- [ ] `src/tools/dispatch.rs:78-183` `invoke_den_tool` — single ~100-arm dispatch match; combined with descriptor tables, tool names looked up in 5+ separate places in the crate.
- [ ] `src/tools/support.rs:490-588` — bespoke HMAC-based confirmation-token scheme hand-rolled as free functions instead of a named `ConfirmationToken` struct with `issue`/`verify` methods.
- [ ] `src/config.rs` (~660 lines) `Config::load()` — one large function reading ~40 env vars, each hand-repeating the same `env::var().unwrap_or_else().parse().unwrap_or_else(warn+default)` pattern; only `parse_bool_env` is factored out.
- [ ] `src/config.rs:230-248` — local closures defined inside `load()` rather than as module-level helpers, inconsistent with rest of file.
- [ ] `src/tools/environment/payloads.rs:45-60` — clones full `Option<Value>` just to read a few fields; `.as_ref()` + shadowing would avoid the clone in a hot path.
- [ ] `src/tools/aliases.rs`, `constants.rs` — alias table and "is builtin" allowlist are hand-maintained matches over the same ~70 tool constants also duplicated in `descriptor/mod.rs`/`dispatch.rs` — same parallel-tables concern restated at the alias layer.
- [ ] `src/profile.rs:56-63` `BearStance::tags_for_bear` builds `Vec<String>` via `format!` every call including a literal `.to_string()` — use a const slice instead.
- [ ] `src/profile.rs:88` `BearProfile` is a `pub type` alias for `BearStance` explicitly marked "deprecated," yet used as the crate's de facto vocabulary throughout `tools/*` — confusing for newcomers reading the deprecation note.
- Overall: well-organized at module/trait-seam level (clear capability traits, consistent `DenError`, good docs), but the "tool registry" concern is spread redundantly across 5+ parallel tables that should be consolidated into one canonical per-tool table or macro.

### den-bearwire (services/den/crates/den-bearwire)
Module structure: `lib.rs` (router) → `auth.rs`, `events.rs` (SSE), `rpc.rs` (JSON-RPC dispatch), `methods/{mod,client,conversation,resource,run,session,tests}.rs`. `run.rs` (1681 lines) and `client.rs` (1032 lines) dominate; `tests.rs` is 2021 lines.
- [ ] `src/methods/run.rs` (1681 lines) — god-file: request parsing, model-resolution preflight, event persistence, run-state machine, and RPC entry points all interleaved — split into `run/model.rs`, `run/persistence.rs`, `run/handlers.rs`.
- [ ] `src/methods/client.rs:322-536` `spawn_continuation_task` — ~215-line closure with 4-deep nested match/loop — extract stream-draining loop into a named async fn.
- [ ] `src/methods/client.rs:675,737,922,971` — `.expect("...outcome should include result row")` encodes an invariant the compiler can't see — model `result` as non-`Option` on those enum variants instead.
- [ ] `src/methods/run.rs:83-138`, `client.rs` — errors represented as ad hoc `(String, Value)`/`json!` tuples with stringly-matched `reason` — replace with an `OperationalOutcomeReason` enum.
- [ ] `src/methods/session.rs:71-81` — near-identical `.as_deref().filter().unwrap_or()` chains for `conversation_runtime_id`/`conversation_external_id`, one returning `String` one `&str` — consolidate.
- [ ] `src/methods/session.rs:124-216`, `run.rs:1130-1223` — duplicate "resolve-or-mint session/conversation id" logic almost verbatim — extract shared `resolve_or_create_session_identity` helper.
- [ ] `src/methods/client.rs:47-115` `PermissionDecisionInput` — 14 near-synonym variants each needing entries in both `normalized()`/`raw()` — use a data-driven table or serde aliases.
- [ ] `src/methods/run.rs` (many sites) — every `BearWireEvent::ephemeral(...)` call followed by 3-4 lines manually setting bear_id/human_id/session_id/run_id (~15 sites) — add a builder taking an `EventContext`.
- [ ] `src/methods/run.rs:1246-1257` — 10+ separate `let x_for_task = x.clone();` bindings before `tokio::spawn` — use one context struct moved once.
- [ ] `src/methods/run.rs:602` `#[allow(clippy::too_many_arguments)]` on a 14-param function — group into a `ToolCallPersistInput` struct.
- [ ] `src/rpc.rs:81-151` — 10x repeated `method_response(...)` dispatch pattern with hand-written message strings — use a macro or `(method_name, handler)` table.
- [ ] `src/methods/mod.rs:85-90` `deserialize_string` is a no-op alias for `deserialize_required_string` — merge or document why both exist.
- [ ] `src/methods/client.rs:146-157` `ClientToolResultRequest::input()` clones 5 fields incl. a `Value` — avoid by consuming `self` or borrowing.
- [ ] `src/methods/run.rs:1148-1158`, `client.rs:189-194` — clones a `String` just for comparison; `.as_deref().unwrap_or(&x)` pattern used correctly elsewhere (`session.rs:71-76`) but inconsistently here.
- [ ] `src/auth.rs:7-21` vs `events.rs:55-62` — duplicated "read Authorization header → strip → trim → validate" pattern — share one combinator.
- [ ] `src/methods/session.rs:429` — `let _ = ...append_bearwire_event(...).await?;` discards `event_sequence`; the only handler in the file that doesn't surface `event_sequence` in its response, inconsistent with siblings.
- [ ] `src/methods/mod.rs:16-32` `initialize_result` hardcodes route strings (`"/bearwire/v1/rpc"`) that don't match actual mounted routes in `lib.rs:11-12` (`/v1/rpc`) — stale/misleading self-description payload.
- [ ] `src/methods/tests.rs` (2021 lines, largest file in crate) — mixes low-level HTTP/SSE mock-server plumbing with high-level RPC assertions — split mock helpers into `tests/support.rs`.
- [ ] `src/methods/run.rs:203-211` `ResolvedRunModel.source: &'static str` is a stringly-typed log-only tag — use an enum with `Display`.
- [ ] `src/methods/client.rs:196-295` `record_web_fetch_approval_from_permission` mixes decision-mapping, GitHub-specific URL-parsing heuristic, and DB persistence in one 100-line function — extract the GitHub-account parsing into a named/documented helper.
- [ ] `src/methods/mod.rs` — no crate/module-level `//!` docs anywhere explaining BearWire's purpose/protocol shape.
- [ ] `src/methods/run.rs:1140`, `session.rs:139` — `"bearwire".to_string()` default-client literal duplicated — hoist to a `const DEFAULT_CLIENT`.
- [ ] `src/methods/client.rs:538-787` vs `789-1032` — `client_tool_result_result`/`client_permission_result_result` are structurally near-identical ~250-line functions (biggest duplication in the crate) — factor shared skeleton into a generic helper.
- Overall: functionally serious, mostly panic-safe, consistent `Result`/`?` usage — but `run.rs`/`client.rs` are oversized god-files with heavy structural duplication (event-context stamping, id resolution, tool/permission-result handling); due for a consolidation pass.

### tools/bear-armature — partial (25k lines, spot-checked + deep read of largest files)
- [ ] `src/main.rs` (14,269 lines / 9,737 non-test) — god-file mixing JSON-RPC transport, browser-bridge MCP server, ACP session/plan/mode state, direct-tool dispatch, CLI arg parsing — split into `browser_bridge.rs`/`session_state.rs`/`plan.rs`/`cli.rs`/`dispatch.rs`.
- [ ] `src/main.rs:2165` `handle_request` — ~900-line single match over JSON-RPC method strings with deeply nested inline error handling — dispatch to per-method handler functions.
- [ ] `src/main.rs:2333,2400,2450` (~6 sites) — identical `write_response(id, Err(json_rpc_error(-32602, ...)))` boilerplate copy-pasted per match arm — extract `invalid_params_response(id, msg)` helper.
- [ ] `src/tools/fs.rs` (2002 lines) — combines file read/write, patch parsing, recursive copy/delete, glob matching, search filtering — split into `patch.rs`/`glob.rs`/`search.rs`.
- [ ] `src/bearwire.rs` (1652 lines) — mixes event-to-legacy-format translation, SSE parsing, RPC transport — move `bearwire_*_to_legacy_*` translators to `legacy_translate.rs`.
- [ ] `src/tools/fs.rs:1860` — hand-rolled recursive glob matcher instead of `glob`/`globset` crate.
- [ ] `src/tools/fs.rs:1546` `parse_simple_unified_patch` — manually parses unified diff format instead of a diff-parsing crate (`similar`/`diffy`) — ~65 lines of risky hand-parsing.
- [ ] `src/tools/fs.rs:1434,1443` `truncate_chars_reverse`/`truncate_chars` manually walk `char_indices` — use combinators or `unicode-segmentation`.
- [ ] `src/approvals.rs:354` `approval_ttl_secs(risk: &str)` matches on string literals instead of a `Risk` enum, despite `LocalToolStatus` (main.rs:315) showing the enum pattern is already used elsewhere.
- [ ] `src/main.rs:427` `normalize_mode`/`session_modes_for_mode`/`mode_config_option` take/return raw `&str` for a small closed set of modes rather than an enum with `Display`/`FromStr`.
- [ ] `src/tools/command_policy.rs` `command_family_key`/`command_policy_for` operate on raw command strings — a `CommandFamily` enum would make the policy table's exhaustiveness checkable.
- [ ] No `thiserror` dependency; crate relies on `anyhow::Error` + hand-rolled `LocalToolError` (main.rs:338) — its `error`/`permission_denied`/`cancelled`/`timeout` constructors (main.rs:1018-1066) duplicate the same `diagnostic: json!({...})` shape 3x — use a builder or enum-per-variant.
- [ ] `src/approvals.rs:346` `now_secs()` swallows `SystemTime` error via `.unwrap_or(0)` with no comment explaining why epoch fallback is safe.
- [ ] `src/approvals.rs:44` `#[allow(dead_code)]` on `ApprovalRecord` derive — audit whether fields are actually unused.
- [ ] `src/main.rs:5940` — stray `#[cfg(test)]` fn defined mid-file, separate from the main `mod tests` block at line 9738 — splits test code across two locations.
- [ ] `src/main.rs:2296-2304` — `session_id.clone()` used twice + `context.clone()` then moved right after; one clone would suffice with reordering.
- [ ] `src/main.rs:2323,2535,2636,2692,2746` — `request.id.clone()` repeated per match arm; bind `id` once up front instead.
- Overall: good low-level discipline (no unwrap/expect/panic outside tests), but suffers from scale — `main.rs`/`fs.rs` are god-files, `handle_request` is a 900-line match full of copy-pasted boilerplate, plus reinvents glob-matching and unified-diff parsing instead of using crates for them.
- **Not yet covered:** `src/tools/mcp.rs` (1028 lines), `src/tools/chrome.rs` (645), `src/tools/git.rs` (601), `src/tools/process.rs` (564), `src/tools/terminal.rs` (531), `src/update.rs` (681), `src/tool_tasks.rs`, `src/json_rpc.rs`, `src/paths.rs`, `build.rs`, and most of the first 2000 / last 4000 lines of `main.rs` were only spot-checked.

### den-web (services/den/crates/den-web) — 40 files, ~16k lines
Largest files: `bear/settings.rs` (2450), `bear/management.rs` (1491, largely dead), `bear/memory.rs` (1212), `v1/mod.rs` (1184), `admin/bears.rs` (1155, partly dead), `admin/oauth_clients.rs` (1091), `observability/chat_proxy_stream.rs` (985).
- [ ] `src/bear/settings.rs` (2450 lines) — god-file mixing routing, bundle import/export (zip/sqlite), session-flash helpers, admin row-mapping, view handlers — split into `settings/bundle.rs`/`models.rs`/`conversations.rs`.
- [ ] `src/bear/management.rs` (1491 lines) — entire file `#![allow(dead_code)]` with stale TODO to delete "during the den-web move" — confirmed-dead handlers still compiled.
- [ ] `src/admin/bears.rs` — same `#![allow(dead_code)]` + stale-TODO pattern.
- [ ] `src/user/account/mod.rs:290` `auth_session.user.clone().unwrap().id` will panic on inconsistent session state; same file at line 325 uses the safe `.as_ref().map().ok_or_else()` pattern — inconsistent within one file.
- [ ] `src/user/settings/email.rs:64,86,118,169,193` — same `auth_session.user.clone().unwrap().id` repeated 5x — extract shared helper returning `Result<_, CustomError>`.
- [x] `src/admin/membership.rs:97,104` `bear_id.expect("checked")` twice, re-deriving an already-validated value instead of an `if let Some` guard/early return. **DONE** (safety batch): replaced with `if let (true, Some(bid)) = ...` guards; `den-web` clippy green.
- [ ] `src/core/s3/mod.rs:37,51,59,68` — four `.expect()` calls in startup config parsing, undocumented as fail-fast-only.
- [ ] `src/v1/mod.rs:826,838,906,919` — `direct_chat_sse_response`/`chat_sse_response`/header-building block all duplicate identical `Response::builder()...text/event-stream` boilerplate 3x — share one builder.
- [ ] `src/admin/oauth_clients.rs:246-376` `add_oauth_client_action` nests entire success path inside `if validation_errors.is_empty()` (130+ lines deep) — invert to early-return error branch.
- [ ] `src/admin/oauth_clients.rs:331-361` — near-identical copy-pasted error-extraction closures differing only by field name — extract `field_error_messages(errors, field)`.
- [ ] `src/bear/settings.rs:475-477` `pretty_json` swallows serialization errors via `unwrap_or_else` with no comment.
- [ ] `src/bear/settings.rs:694-713` `build_bear_bundle` chains 5 nearly identical `.map_err(...CustomError::System(format!(...)))` closures.
- [ ] Crate-wide — 35+ sites of `.map_err(|err| CustomError::Database/System(format!(...)))` for errors that already have working `From` impls in `den-http/src/errors.rs` — most could just use `?`.
- [ ] `src/observability/chat_proxy_stream.rs:473` `.expect("terminal error serializes")` in production stream-polling error-path code, not test code.
- [ ] `src/bear/settings.rs:452-473` `slug_base` hand-rolls slug sanitization, likely duplicating a slugify crate or existing bear-creation logic.
- [ ] `src/bear/settings.rs:715-749` `bear_bundle_entry_name` — security-relevant path-traversal filter (`"__MACOSX/"`, `".."`) buried inline in an anonymous closure inside a formatting helper — should be a named, tested, documented function.
- [ ] `src/bear/settings.rs:663,667` — clones `serde_json::Value` trees unnecessarily when building a manifest that's immediately serialized and dropped.
- [ ] `src/v1/mod.rs:1038,922` `chat_send_inner`/`chat_send_native_inner` — two large "inner" functions threading 7-8 positional args — use a request-context struct.
- [ ] `src/lib.rs:369-449` `render_template` handles 4 rendering modes via nested if/else with duplicated `CustomError::Render(format!(...))` — use a match over a parsed `TemplateTarget` enum.
- [ ] `src/lib.rs:314-323` `/metrics` handler uses a fallible `Response` builder for a fixed content-type/body that can't really fail — overkill.
- [ ] `src/bear/memory.rs:26-46` `is_ephemeral_progress_status`/`strip_ephemeral_status_suffixes` — loop-based suffix stripping with no comment on termination guarantee.
- [ ] Crate-wide naming — inconsistent handler suffixes (`_view`/`_get`/`_post`/`_action`/`_response`/`_inner`) with no single convention.
- [ ] `src/bear/settings.rs:26-61` — 15+ item multi-line `use` block from 4+ submodule paths — signals the file's responsibilities should be split.
- Overall: functionally organized, error propagation via `CustomError`+`?` mostly correct, but copy-paste repetition (SSE builders, error-mapping closures), dead-code files kept alive by `#![allow(dead_code)]`, inconsistent `Option<SessionUser>` handling, and 2-3 oversized files needing splits.
- **Not yet covered:** `src/admin/users.rs`, `src/onboarding.rs`, `src/bear/profile.rs`, `src/user/settings/mod.rs`, `src/status.rs`, `src/bear/create_support.rs` (skimmed only).

### den-service (services/den/crates/den-service, ~15k lines) — most files covered
- [ ] `src/conversation/events.rs` (1317 lines) — mixes canonical record construction, dedup logic, spawn/persist plumbing, and 8 `*_projection` builders — split into `canonical.rs`/`projections.rs`.
- [ ] `src/conversation/events.rs:984-1203` — 8 near-identical `memory_*_projection` free functions (each 7-9 positional params) — replace with one `Payload -> Projection` conversion or builder.
- [ ] `src/conversation/persistence.rs` (1181 lines) — cohesive but large; split into `read.rs`/`write.rs`.
- [ ] `src/bears/db.rs` (956 lines) — raw SQL wrappers for one table family; split bear-CRUD vs membership/binding queries.
- [ ] `src/conversation/persistence.rs:853-996` `append_message` — manually wraps ~10 separate `sqlx::Error`s in `DenError::Database(format!(...))`; same pattern repeats 71x in this file.
- [ ] `src/prompt_memory_block_store.rs:270-272` already has a clean `db_decode(field)` closure-factory pattern that `persistence.rs` should adopt instead of duplicating format strings.
- [ ] `src/bears/db.rs:51` uses idiomatic `.map_err(Into::into)` while `persistence.rs` almost never does — inconsistent error-conversion style within the crate.
- [x] `src/tool_turns.rs:298,354,398,747` — 4 identical `DenError::System("... lock poisoned".to_string())` literals — extract a `poisoned(what: &str)` helper. **DONE** (duplication batch): added `poisoned_lock(what)` and replaced Result-returning lock error literals; `den-service` clippy green.
- [ ] `src/conversation/persistence.rs:889-892,955-959` — manual rollback-then-return-error repeated at every failure branch inside one transaction — needs a `TxGuard`/`?`-based combinator.
- [ ] `src/conversation/events.rs`, `message_types.rs`, `bears/managed_blocks.rs`, `bifrost_governance.rs` — 10 enums hand-implement `as_str(self) -> &'static str` instead of `Display`/`AsRef<str>`.
- [ ] `src/memory_proposals.rs:7-9` — pointless doubled brace nesting in a `use` statement — unreviewed rustfmt/import churn.
- [ ] `src/bifrost.rs:15,18,25,28` — 4 struct fields carry `#[allow(dead_code)]` — remove fields or investigate why suppressed.
- [ ] `src/conversation/events.rs:996-1042` — callers clone `title`/`source_profile`/`status` multiple times per call since both the payload struct and `format!` summary need owned copies — take `&str` instead (as `pair_reflection/mod.rs`'s `CreatePairReflectionRun<'a>` already does).
- [ ] `src/tool_turns.rs:130-136,518-523` — converts `PendingToolTurn`/`ToolTurn` to `SettledToolResult` field-by-field with 5-6 clones — add a `From<&ToolTurn> for SettledToolResult` impl.
- [ ] `src/tool_turns.rs` — 32 `.clone()` calls, highest density in the crate.
- [ ] `src/conversation/events.rs:1075-1203` — projection builders take 6-9 positional args of the same types (`String`, `Option<String>`, `Uuid`) — error-prone at call sites, should take a payload struct.
- [ ] `src/bears/db.rs:13-21` (`BearParams<'a>`), `pair_reflection/mod.rs:32-48` (`CreatePairReflectionRun<'a>`) show the crate's good borrowed-param-struct pattern — projection/proposal constructors should follow suit.
- [ ] `src/bifrost.rs:300` `.expect("reqwest client")` in `BifrostClient::new` panics on construction — undocumented as intentional startup-only panic.
- [ ] `src/conversation/events.rs:1206-1211` — two `.expect()`s assume `ProjectionEvent` always serializes to a JSON object — safe but undocumented invariant.
- Overall: disciplined about `Result`/`?` (virtually no stray unwrap/panic), some genuinely clean modules (`secrets.rs`, `turn_controller.rs`, `recall/query.rs`, `prompt_memory_block_store.rs`), but `conversation/events.rs`/`persistence.rs` carry disproportionate duplication.
- **Not yet covered:** `src/bears/managed_blocks.rs` (623 lines), `src/client_sessions.rs` (478 lines), `src/bears/prompt_fragments/*`, `src/recall/qdrant.rs`, `src/model_selection.rs`, `src/archived_conversations.rs`.

---

### tools/bear-armature — approvals.rs, bearwire.rs, tools/mcp.rs
- [ ] `src/approvals.rs:44` `ApprovalRecord` has `#[allow(dead_code)]` on the whole struct masking which fields are unused.
- [ ] `src/approvals.rs:163,186,252,263,732` — 5 public methods carry `#[allow(dead_code)]` — if unused outside tests, remove or `#[cfg(test)]`-gate instead of silencing.
- [ ] `src/approvals.rs:100-345` `ApprovalCache` mixes cache lookup/TTL pruning/file persistence in one impl block — extract persistence into a submodule.
- [ ] `src/approvals.rs:308-310` `is_allowed_for_target` mutates (`retain`) the shared map on every read call — surprising side effect for a "read" method.
- [ ] `src/approvals.rs:648-680` `parse_permission_decision` returns `Result` but never produces `Err` — vestigial wrapper misleads callers.
- [ ] `src/approvals.rs:377,362` `command_family_workspace_fingerprint`/`command_workspace_fingerprint` near-duplicates — share a helper.
- [ ] `src/approvals.rs:454-488,545-639` `candidate_approval_scopes`/`permission_options_for_context` independently hand-encode the same priority order — risk of drift, use a shared ordering table.
- [ ] `src/approvals.rs:70` `default_approval_scope_kind` calls `.as_str().to_string()` on an already-`'static str` — unnecessary allocation.
- [ ] `src/approvals.rs:24-28` `ApprovalTarget` has 3 parallel `Option` fields (path/url/command) with no doc comment that they're mutually exclusive — model as an enum instead.
- [ ] `src/bearwire.rs` (1652 lines) — god-module: RPC transport, SSE parsing, legacy-event translation, tool-summary heuristics, prompt-loop orchestration all interleaved — split into `rpc.rs`/`sse.rs`/`legacy_translate.rs`/`prompt_loop.rs`.
- [ ] `src/bearwire.rs:321-505` `handle_prompt` — ~185-line function combining session-open/model-sync/run-start/polling/diagnostics/timeout checks — split into named steps.
- [ ] `src/bearwire.rs:1000-1293` `handle_bearwire_event` — one large match over ~20 event-type arms, some 40+ lines — extract per-arm handler functions.
- [ ] `src/bearwire.rs:19-40` `generic_tool_summary`/`generic_tool_summary_for_tool` — confusingly similar names with overlapping checks — consolidate or rename.
- [ ] `src/bearwire.rs:707` `response.text().await.unwrap_or_default()` silently swallows body-read errors into an empty string used in error messages.
- [ ] `src/bearwire.rs:766-773` — hand-rolled SSE frame splitting via byte-scanning, reimplementing what `eventsource-stream` crate provides; O(n) rescans per chunk.
- [ ] `src/bearwire.rs` (general) — JSON traversal via chained `.get().and_then().unwrap_or()` everywhere with no typed intermediate structs — introduce `#[derive(Deserialize)]` structs for recurring event shapes.
- [ ] `src/tools/mcp.rs:140-153,756-799` `summarize_mcp_server_param`/`mcp_client_tool_descriptor` build near-identical JSON shapes by hand in two places — share a builder/struct.
- [ ] `src/tools/mcp.rs:466-532` `configure_session` — 4 near-identical `json!` server-summary blocks duplicated across success/error branches — extract `source_summary_json(...)`.
- [ ] `src/tools/mcp.rs:400-558` `configure_session` — 150+ line function mixing discovery/logging/descriptor-building/bookkeeping; ~15 scattered `if crate::bear_debug_verbose() { eprintln!(...) }` calls hurt readability.
- [ ] `src/tools/mcp.rs:742-754` `bearer_auth_headers` uses fully-qualified `std::collections::HashMap` despite `HashMap` already imported at top — inconsistent.
- [ ] `src/tools/mcp.rs:860-874` `mcp_tool_result_content` silently drops non-text content items with no indication in output.
- [ ] `src/tools/mcp.rs:377-397` `sanitize_name_part` — undocumented custom slug function.
- [ ] `src/tools/mcp.rs:190-291` `parse_acp_mcp_servers` — two near-identical `Err(anyhow!(...))` branches for unsupported transports — share one message template.
- [ ] `src/tools/mcp.rs:110-138,190-208` — `summarize_acp_mcp_servers_param`/`parse_acp_mcp_servers` both re-fetch the same `mcpServers`/`mcp_servers` key-fallback independently — share an accessor.
- [ ] `src/tools/mcp.rs` (general) — `anyhow!("...")` used throughout for distinct structured failure modes; a `thiserror` enum would let callers distinguish failure kinds.
- Overall: all three files functionally solid and well-tested, but share a "stringly-typed JSON plus anyhow-string-errors" style producing long deeply-nested single-purpose functions; would benefit most from typed event/config structs and splitting `bearwire.rs`.

### tools/bear-armature — chrome.rs, update.rs, git.rs, process.rs, terminal.rs, tool_tasks.rs, json_rpc.rs, adapter_env.rs, web.rs, command_policy.rs, paths.rs, rtk.rs
- [x] `src/tools/process.rs:420`, `src/tools/terminal.rs:31`, `src/tools/rtk.rs:65` — identical `output_excerpt` function copy-pasted verbatim in 3 places — share one helper. **DONE** (duplication batch): extracted shared `tools::common::output_excerpt` for `process.rs`, `terminal.rs`, and `rtk.rs`; `bear-armature` cargo check green. Note: strict clippy is currently blocked by broader pre-existing `bear-armature` lints outside this extraction batch.
- [x] `src/tools/process.rs:383-393`, `src/tools/terminal.rs:116-126` — identical `rtk_available()` duplicated verbatim. **DONE** (duplication batch): extracted shared `tools::common::rtk_available`; `bear-armature` cargo check green. Note: strict clippy is currently blocked by broader pre-existing `bear-armature` lints outside this extraction batch.
- [ ] `src/tools/process.rs:395-418`, `src/tools/terminal.rs:90-114` — nearly identical `reduce_*_output_with_rtk` wrappers — extract shared generic helper.
- [ ] `src/tools/process.rs`, `src/tools/terminal.rs` — both build `effective_command`/`effective_args` for rtk-wrap case with same logic — extract `build_rtk_invocation(...)`.
- [ ] `src/tools/process.rs:507-564` vs `src/tools/terminal.rs:436-465` — "looks secret-like" env-key check (`SECRET`/`TOKEN`/`PASSWORD`/`KEY`) copy-pasted rather than shared.
- [ ] `src/tools/process.rs:126-381` `handle_process_run` — ~255-line function with 2 near-identical `json!` result blocks (timeout vs success paths) — unify into one result-builder.
- [ ] `src/tools/terminal.rs:128-372` `handle_terminal_run_command` — same shape/problem, duplicate large `json!` blocks (291-312 vs 350-371).
- [ ] `src/update.rs:173-219` `run_update` mixes OS-gating/fetch/confirm/download/verify/3-way install dispatch — extract install-mode dispatch to its own function.
- [ ] `src/tools/chrome.rs` (645 lines) — combines CDP process management, WebSocket session handling, event buffering, header redaction, 6 tool handlers — split into `chrome/capability.rs`/`session.rs`/`handlers.rs`.
- [ ] `src/tools/chrome.rs:84,158-162,237,613-620` — inconsistent use of `unwrap_or`/`unwrap_or_default` on JSON extraction, silently defaulting missing fields rather than erroring, with no consistent policy on when it's an error vs default.
- [ ] `src/tools/git.rs`, `src/tools/process.rs` — 15+ near-identical ad hoc `anyhow!("... args missing ...")` strings instead of a shared `missing_field(tool, field)` helper.
- [ ] `src/tools/web.rs:83-137` — two nearly identical URL-validation functions, one only reachable under `#[cfg(test)]` and validating different (denylist) semantics — looks like dead/legacy code, remove or merge.
- [ ] `src/tool_tasks.rs:11,178` `#[allow(dead_code)]` on `ToolTaskRecord` fields and `list_for_session` — remove if truly unused rather than silencing.
- [ ] `src/tools/git.rs:90,133,192,217,270` — repeated inline `paths.iter().map(...to_string_lossy().to_string()).collect()` pattern duplicated 5x — extract `paths_as_strings(&[PathBuf]) -> Vec<String>`.
- [ ] `src/tools/chrome.rs:439` `ChromeCapability::detect().clone()` clones the whole enum just to match on it — match by reference instead.
- [ ] `src/tools/process.rs:213-220`, `src/tools/terminal.rs:174-181` — both clone `command_args` then separately build `effective_args` — avoid double allocation.
- [ ] `src/tools/chrome.rs:369` `detect_local_chrome_executable` vs `:413` `resolve_executable` — naming doesn't distinguish "probe PATH+known locations" from "validate one candidate".
- [ ] `src/tools/command_policy.rs:120` `process_command_preferred` — misleading name; actually gates a narrow read-only-utility allowlist — rename to something like `is_process_run_safe_default`.
- [ ] `src/paths.rs:36` `resolve_requested_tool_path` vs `:26` `normalize_requested_tool_path` — both take a "requested tool path" but one requires absolute (errors otherwise), the other silently joins onto cwd if relative — naming doesn't signal the difference.
- [ ] `src/tools/adapter_env.rs:14,207` `collect_bear_environment` vs `handle_bear_environment` — the latter is a trivial pass-through wrapper with no added value — remove and point callers directly at the former.
- [ ] `src/tools/rtk.rs:56` `rtk_reducer_enabled` reimplements a boolean-env-var parser also present in `update.rs:18` `env_with_fallback` — consolidate into a shared `env` utils module.
- [ ] `src/tools/chrome.rs:82-92` `ChromeState::push_event` clones the full event `Value` unconditionally even when it won't match `Network.*` — clone only when actually pushed.
- [ ] `src/tool_tasks.rs:194-231` `ToolTaskPhase::as_str` — manual Display-like impl; implement `std::fmt::Display` instead so callers can use `{phase}` directly.
- `src/json_rpc.rs` — clean and idiomatic, no notable issues (good use of oneshot channels, snapshot structs, bounded VecDeque — same pattern independently reimplemented in chrome.rs, missed reuse opportunity between the two).
- Overall: generally solid and defensive (careful path/URL/secret validation), but significant copy-paste duplication between `process.rs`/`terminal.rs` (rtk wrapping, truncation, env validation) and several handlers building near-duplicate success/timeout JSON blocks.

### den-runtime — native_runtime/turn.rs, gateway_events.rs, reflection/conductor.rs, agent_loop/session_stream.rs
- [ ] `src/native_runtime/turn.rs:355` `build_session` takes 18 positional args (many `Option<&str>`/`Option<&Value>`) — invites swapped-argument bugs.
- [ ] `src/native_runtime/turn.rs:274-275,1032-1033,1076-1077` — `serde_json::from_str(...).unwrap_or_else(|_| Value::Object(Default::default()))` duplicated 3+ times — extract `parse_args_or_empty_object`.
- [ ] `src/native_runtime/turn.rs:211-339` `record_native_client_tool_result` — ~130-line function mixing DB writes/session mutation/canonical persistence/status mapping.
- [ ] `src/native_runtime/turn.rs:233-239,254` — avoidable clones (`content.clone()` before reuse; `.cloned()` on a whole `ChatToolCall` just to read one field).
- [ ] `src/native_runtime/turn.rs:686` `SESSION_STORE.insert(session.clone())` clones the entire session (messages/tools/budget state) just to keep a local copy.
- [ ] `src/native_runtime/turn.rs:56-147` `render_host_context_for_model` — two near-duplicate line-building blocks repeating kind/delivery/persistence extraction 3x.
- [ ] `src/native_runtime/turn.rs` (4 call sites) — `Arc::new(request.config.clone())` re-wraps a cloned `Config` on every turn instead of caching once in `NativeRuntimeDeps`.
- [ ] `src/native_runtime/turn.rs:1058-1126` vs `session_stream.rs:257-303` — `DenToolInvocationContext` construction duplicated almost verbatim in both files — extract shared constructor.
- [ ] `src/native_runtime/turn.rs:942-953` / `session_stream.rs` — "search messages in reverse for matching tool_call_id" logic duplicated across files — consolidate as an `AgentLoopSession` method.
- [ ] `src/native_runtime/turn.rs:1006-1009` vs `session_stream.rs:305-316` — "is this call the web_fetch Den tool" check scattered across both files with different formulations.
- [ ] `src/native_runtime/turn.rs:1285-1927` — ~650 lines (a third of the file) are inline `#[cfg(test)]` — move to `turn/tests.rs`.
- [ ] `src/native_runtime/turn.rs:824-935,937-1283` — `start_native_profile_turn_event_stream`/`continue_native_client_turn_event_stream` are ~110/~150-line "entire turn lifecycle in one function" — split into `assemble`/`persist_prompt`/`wrap_stream`.
- [ ] `src/gateway_events.rs:19-92,822` `GatewayEvent` enum — 12 variants converted to wire format by hand in one 240-line match instead of `#[derive(Serialize)]` with `#[serde(tag = "type")]`.
- [ ] `src/gateway_events.rs:350-495` `native_provider_tool_request_event_with_args` — ~145-line god function (tool-name resolution, validation, approval-id derivation, event construction) — decompose.
- [ ] `src/gateway_events.rs:668-750` — 4 nearly identical field-lookup helpers (`tool_call_value`/`tool_call_id`/`tool_call_name`/`tool_call_args_raw`) — replace with one generic `lookup_field(&[&Value], &str)`.
- [ ] `src/gateway_events.rs:263-332` — text-extraction functions hardcode long parallel lists of JSON pointer strings, nearly identical between `reasoning` and `content`/`text` — table-driven instead.
- [ ] `src/gateway_events.rs:509-644` `ToolCallAccumulator` mixes 3 provider wire formats in one struct; `observe`/`observe_openai_tool_call_delta` duplicate buffer/emit bookkeeping (~20 lines) — extract `finish_and_emit`.
- [ ] `src/gateway_events.rs:1063` `preview_str_truncated` — byte-slice `&s[..max]` can panic on multi-byte UTF-8 boundaries — use `char_indices`/`floor_char_boundary`.
- [ ] `src/gateway_events.rs:1-17` — imports mix `den_core::client_tools` and `den_core::tools::descriptor` for one conceptual "tool descriptor" concern split across two modules — blurs the file's adapter boundary.
- [ ] `src/gateway_events.rs:785-820` `gateway_event_adapter_type`/`gateway_event_has_visible_output` — two separate matches over the same enum for related classification — unify or make inherent methods.
- [ ] `src/reflection/conductor.rs` (whole file, 1354 lines) — 4 lanes each reimplement the identical claim/complete/fail/list-queued/worker-loop quintet near-verbatim (4x) — biggest idiomatic issue in the crate, should be a generic `ReflectionLane` trait.
- [ ] `src/reflection/conductor.rs:210-246,818-851,1081-1116,1299-1334` — 4 structurally identical worker-loop functions — collapse to one generic `run_lane_worker_loop<F>`.
- [x] `src/reflection/conductor.rs:734-738` `native_curate_llm_briefing_enabled` reads env var on every call rather than caching via `OnceLock`/`LazyLock` (pattern used elsewhere, e.g. `turn.rs:54`). **DONE** (structural batch 6): cached via a function-local `OnceLock<bool>`.
  - (`parse_context_compact_input` returning `Result<_, String>` was left as-is: the String is deliberately the DB failure message passed to `mark_context_compact_failed(&error)`, so `DenError` would only force re-stringification at the call site.)
- [ ] `src/reflection/conductor.rs:1216-1231` `parse_context_compact_input` returns `Result<_, String>` instead of `DenError`, inconsistent with rest of file.
- [x] `src/reflection/conductor.rs:1336-1354` `row_from_sql` uses `row.get(...)` (panics on missing/wrong-typed column) instead of `try_get`, inconsistent with rest of file's `Result` style. **DONE** (structural batch 6): derived `sqlx::FromRow` on `ReflectionRunRow` and converted all 17 call sites from `sqlx::query(...).map(row_from_sql)` to `sqlx::query_as::<_, ReflectionRunRow>(...)` (decodes via `try_get`, propagates errors); deleted the manual mapper and the now-unused `sqlx::Row` import. Net −19 lines; clippy `--all-targets -D warnings` clean.
  - Also observed while here (NOT changed — flag for follow-up): `run_memory_curate_worker_loop` handles a per-bear run error with `?`, which terminates the whole worker loop, whereas the other 3 lanes (archive_harvest/recall_index/context_compact) log-and-continue. Likely a latent bug — a single failing curate run stops the worker until restart.
- [ ] `src/reflection/conductor.rs:307-312,808` — `format!("bear:{}:lane:{}", ...)` "scope id" pattern duplicated rather than centralized as a constructor.
- [ ] `src/agent_loop/session_stream.rs:541-845` `poll_next` — ~300-line hand-rolled state machine with 7 `Option` pending-state fields — textbook god function, candidate for `async-stream` or an explicit `enum State`.
- [ ] `src/agent_loop/session_stream.rs:103-127` `SessionTrackingStream` — 18 fields mixing stream state, DB/config deps, and pause bookkeeping — split into "dependencies" vs "in-flight state" structs.
- [ ] `src/agent_loop/session_stream.rs:257-303` `server_tool_context()` calls `self.store.get(&self.session_key)` 4 separate times to read different fields off the same session — one `get` + 4 field reads would do.
- [ ] `src/agent_loop/session_stream.rs:173-185` `accumulated_tool_calls` clones every tool-call arg string on every call, invoked multiple times per poll cycle.
- [ ] `src/agent_loop/session_stream.rs:318-341,664-687` `web_fetch_permission_target` — double-clones the same JSON blob within a few lines (`arguments.clone()` then cloned again into `arguments_value`).
- [ ] `src/agent_loop/session_stream.rs:384-479` `begin_server_tool_execution` — ~95-line function cloning 9 separate values to move into one async block; split into 2-3 named async steps.
- [ ] `src/agent_loop/session_stream.rs` — no doc comments on any `SessionTrackingStream` public method explaining the pause/resume protocol, which is genuinely non-obvious.
- [ ] `src/agent_loop/session_stream.rs:213-238` `remove_recent_server_tool_chain_from_session` — 3-level nested conditionals doing index arithmetic (`tool_index - 1`) instead of a named "last exchange" accessor on `AgentLoopSession`.
- Overall: all four files show the same pattern — sound async/Postgres plumbing undermined by copy-pasted control flow, oversized multi-responsibility functions, and repetitive `.clone()`/`unwrap_or_else` instead of shared helpers or trait-based abstraction. None incorrect, but each file would shrink 20-40% with straightforward extraction.

### den-runtime — conversation/events.rs, agent_loop/context.rs, agent_loop/budget.rs, llm/stream.rs, runtime/turn_state.rs, runtime/compaction/lifecycle.rs, turn_waits.rs
- [ ] `src/conversation/events.rs:16-30,489` — 8-field builder-style struct plus an 8-arg constructor duplicating it — use a builder pattern.
- [ ] `src/conversation/events.rs:240-270` `tool_request` — 9 positional params, several `Option<String>`/`bool` in a row — easy to transpose, use a request struct.
- [ ] `src/conversation/events.rs:872-1082` — 8 near-identical `*_projection` free functions differing only in field mapping — needs a builder or macro (echoes den-service finding).
- [ ] `src/conversation/events.rs:441-458` `canonical_record_already_persisted` silently swallows JSON parse errors rather than logging — dedup could silently degrade to always-append.
- [ ] `src/conversation/events.rs:1086,1089` — two `.expect()` calls on serialization in library code ("projection event should serialize") — should propagate `Result`.
- [ ] `src/conversation/events.rs` — mixes 3 concerns (canonical record model, persistence I/O, projection/event catalog) — split into `canonical.rs`/`projections.rs` (echoes den-service's own `events.rs` finding — this is a *different* `events.rs` in den-runtime).
- [ ] `src/agent_loop/context.rs` — filename doesn't match contents (actually transcript reconstruction/pruning logic) — rename to `transcript.rs`.
- [ ] `src/agent_loop/context.rs:97-210,214-301` `reconstruct_transcript_messages`/`repair_tool_call_message_chain` — overlapping responsibility resolving orphaned/out-of-order tool messages, deeply nested with scattered `continue` — share a helper, split into named sub-functions.
- [ ] `src/agent_loop/context.rs:401,405` `load_transcript_grouping_rows` — `.unwrap_or_default()` on a DB fetch error silently returns empty transcript instead of propagating, masking real failures as "no history."
- [ ] `src/agent_loop/budget.rs:29-81` hand-rolled `impl Default` for `ToolCallBudgetUsage` — `#[derive(Default)]` would suffice (all fields `u32`).
- [ ] `src/agent_loop/budget.rs:43-65` `count_for`/`limit_for` — two parallel manual matches over the same 7-variant enum — use an array/EnumMap indexed by variant.
- [ ] `src/agent_loop/budget.rs:262-305` `classify_tool_budget_class` — giant hardcoded string-literal match of tool names to classes, fragile coupling — use a lookup table or tool-definition attribute.
- [ ] `src/agent_loop/budget.rs:445-468,499-514` — same 7-element class-ordering array duplicated verbatim in two functions — extract `const ALL_CLASSES`.
- [ ] `src/llm/stream.rs:23-29,401-408` — two near-duplicate accumulator structs with near-identical SSE-line-parsing loops across 3 functions — needs one shared SSE-line iterator helper.
- [ ] `src/llm/stream.rs:553,674,627` — 3 separate `from_utf8(...).map_err(...)` blocks with copy-pasted error text — extract `decode_utf8_or_system_error`.
- [ ] `src/llm/stream.rs:592,624,650,658` — 4 public entry points with overlapping purposes and confusingly similar names, only one has a doc comment distinguishing them.
- [ ] `src/runtime/turn_state.rs:54-85` `turn_state_json`/`turn_state_from_sources` — builds deeply nested `Value` by hand instead of typed `#[derive(Serialize)]` structs; schema versioning is stringly-typed.
- [ ] `src/runtime/turn_state.rs:183-317,395-405` — 4 `*_domain_json` functions repeat `.map(...).unwrap_or(Value::Null)` 10+ times — same untyped-JSON anti-pattern.
- [ ] `src/runtime/turn_state.rs:130-156` `classify_autonomous_final_response` — brittle heuristic substring-matching on assistant free text (comment admits it's tech debt to replace).
- [ ] `src/runtime/turn_state.rs:194-196` — `plan_id`, `id`, `root_id` all set to the same value with unclear distinct purpose.
- [ ] `src/runtime/turn_state.rs` — mixes JSON wire-schema rendering, autonomous-execution gating logic, and text classification heuristics in one 521-line file.
- [ ] `src/runtime/compaction/lifecycle.rs:120-246` `run_compaction_job` — large function doing decision selection/artifact writing/event building/side-effect enqueueing inline — split into `decide()`/`persist_decision()`/`record_event()`.
- [ ] `src/runtime/compaction/lifecycle.rs:301-321` `prepare_turn_compaction` — labeled "back-compat alias" with 3 duplicate match arms all calling the same function — collapse to `_ => ...`.
- [ ] `src/runtime/compaction/lifecycle.rs:171-213` — 3 levels of nested nontrivial logic assigning a 3-tuple — extract "Active mode: summarize + persist artifact" into a named helper.
- [ ] `src/turn_waits.rs:72-162,164-445` `persist_surface_obligation_transactionally`/`persist_bearwire_tool_call_wait_transactionally` — duplicate an entire tx block (begin/update state/select-or-insert step/update to waiting) verbatim — extract `ensure_waiting_turn_step(...)`.
- [ ] `src/turn_waits.rs:52-70` `obligation_from_row` hand-maps 15 columns off a raw `PgRow` — should derive `sqlx::FromRow`; called against two different query shapes (hidden coupling risk).
- [ ] `src/turn_waits.rs:256-354` — 3-way branching (UPDATE-RETURNING / INSERT-ON-CONFLICT-RETURNING / separate INSERT) with the `RETURNING` column list repeated verbatim 3x — extract a shared constant.
- [ ] `src/turn_waits.rs:10-26` `PersistToolCallWaitInput` — several fields typed `&'a Option<String>` rather than the more idiomatic `Option<&'a str>`, forcing callers to hold owned `Option<String>`s.
- [ ] `src/turn_waits.rs` — despite the name, directly interleaves BearWire event construction with DB persistence — distinct concerns, could split.
- Overall: all seven files functionally solid (consistent `DenError`/`Result`, no stray unwrap/panic in hot paths) but show frequent copy-paste duplication and heavy reliance on untyped `serde_json::Value` construction where typed structs would give compile-time safety.

### den-runtime — recall/qdrant.rs, recall/temporal.rs, recall/indexer.rs, recall/reconcile.rs, memory/curation.rs, bears/context_composition.rs, bears/provision.rs
- [ ] `src/recall/qdrant.rs:73-267` — every method builds its own url/error-string boilerplate (7x near-identical `format!` + status-check blocks) — extract `send_json`/`check_status` helper.
- [ ] `src/recall/qdrant.rs:78,90,109,136,163,202` — all errors are `DenError::System(String)`, no distinction between network/non-2xx/parse failure.
- [ ] `src/recall/temporal.rs:71` `type Match = (Option<..>, Option<..>, bool, usize, usize)` — positional 5-tuple threaded through 7 functions — use a named struct.
- [ ] `src/recall/temporal.rs:262-289` `match_in_month_or_year` — densest/least readable function, nested Options with unexplained arithmetic — split into two helpers.
- [ ] `src/recall/indexer.rs:74-174` `index_record` — 100-line function mixing indexability check/diffing/embed+upsert/pruning — split into `diff_chunks`/`embed_and_upsert`/`prune_stale`.
- [ ] `src/recall/indexer.rs:113,129` — avoidable clones of embed text and embedding vectors that could be moved instead.
- [x] `src/recall/reconcile.rs:34-45` `HeadRow` is a 9-tuple type alias with a comment per field instead of a `#[derive(FromRow)]` struct.  **DONE** (structural batch A): converted to `#[derive(sqlx::FromRow)]` + `query_as` (pure decoding swap, SQL unchanged); verified clippy `--all-targets -D warnings` clean.
- [ ] `src/memory/curation.rs:205-283` `sqlite_proposal_to_row` — ~15-step manual `.get().and_then().unwrap_or().to_string()` chain — replace with a `#[derive(Deserialize)]` struct.
- [x] `src/memory/curation.rs:212` `Uuid::parse_str(...).unwrap_or_else(|_| Uuid::new_v4())` silently manufactures a random id on parse failure instead of surfacing an error — risks hiding data corruption. **DONE** (safety batch): SQLite proposal row conversion is now fallible and returns `DenError::Parsing` on invalid stored proposal ids; callers propagate the error; `den-runtime` clippy green.
- [ ] `src/memory/curation.rs:1-22` — every public fn takes unused `_pool`/`_config` params (7 occurrences) — vestigial signature clutter.
- [ ] `src/memory/curation.rs:130` `get_proposal` fetches up to 500 proposals and linear-scans for one by id instead of a direct lookup.
- [ ] `src/memory/curation.rs:285-303` `sqlite_observation_to_row` fabricates `id: Uuid::new_v4()` and hardcodes `salience: "normal"` rather than reading from source — undocumented whether intentional.
- [ ] `src/bears/context_composition.rs:149-155` `instructions_heading` duplicates `RoleContracts::get`'s match arms with different strings — consolidate into a `BearProfile` method.
- [ ] `src/bears/context_composition.rs:208-226` `default_role_contracts_for_bear` — 200+ word prose blocks inline in a constructor function, hard to scan for logic.
- [ ] `src/bears/provision.rs:16-22` `provision_bear_if_configured` — name implies a conditional that no longer exists; one-line pass-through to `provision_bear_native`.
- [ ] `src/bears/provision.rs:100-102` — `let _ = ...mark_bear_profile_binding_failed(...).await;` silently discards a second failure's Result with no log.
- [ ] `src/bears/provision.rs:120-160` `provision_missing_bear_profiles_native` — trivial wrapper, same dead-indirection pattern as above, suggests leftover abstraction from a removed migration path.
- Overall: solidly structured with good module docs, but leans hard on stringly-typed `DenError::System(String)`, repeats HTTP boilerplate in qdrant.rs, and uses manual `Value` extraction chains where typed structs would be more idiomatic.

### den-runtime — runtime/role.rs, conversations.rs, compaction/{mod,grouping,summarize,artifact_store}.rs, compaction_store.rs, bearwire_projection/{mod,wire}.rs
- [ ] `src/runtime/role.rs:224` `.expect("client turn lifecycle runtime requires cancellation registry")` panics in library code — should be a typed `DenError` variant.
- [ ] `src/runtime/role.rs:20,40,95,118` — four parallel hand-written `as_str()` match blocks — use a macro or `strum` derive.
- [ ] `src/runtime/role.rs:331-333` `RoleTurnGuard.guard` field is `pub`, defeating the point of wrapping `ActiveTurnGuard` — make private.
- [ ] `src/runtime/conversations.rs:106-120,155-169` `runtime_messages_top_array`/`runtime_conversations_top_array` — near-duplicate "unwrap array from one of several keys" logic — share a helper.
- [ ] `src/runtime/conversations.rs:171-181` `truncate_runtime_message` duplicates `summarize.rs:207-219`'s `truncate_chars` almost exactly — consolidate into one shared utility (3rd copy of this pattern in the crate, see also `key_memory_projection.rs`).
- [ ] `src/runtime/conversations.rs` — mixes plain data types, untyped `Value`-scraping helpers, and compaction-related grouping types in one 297-line file with a generic name.
- [ ] `src/runtime/compaction/mod.rs:56-115` `semantic_groups_from_runtime_messages` duplicates classification logic that also exists (more thoroughly) in `grouping.rs::classify_non_tool_row` — risk of drift.
- [x] `src/runtime/compaction/mod.rs:204-238` `merge_iterative_summary` uses `format!("{:?}", group.kind)` (Debug) to build a persisted label — fragile, needs explicit `Display`/`as_str()` on `RuntimeSemanticGroupKind` (recurs in `artifact_store.rs:131` and `compaction_store.rs:85-97,131` — 4 occurrences of this anti-pattern). **DONE** (typing batch): added explicit `as_str()` helpers for semantic group kind, compaction trigger, and compaction event status; compaction summaries, artifact persistence, event persistence, and event hashing now avoid Debug strings. Verified no direct `format!(...{:?}...)` persisted-string patterns remain under runtime modules; `den-runtime` clippy green.
- [ ] `src/runtime/compaction/mod.rs:240-244` `push_unique` duplicated verbatim in `summarize.rs:222-228` (3rd copy across the crate) — needs one shared helper.
- [ ] `src/runtime/compaction/grouping.rs:109-124,132-155` `tool_call_id_from_row`/`is_approval_interaction_row` re-parse the same JSON payload multiple times per row via redundant `try_from` calls instead of parsing once.
- [ ] `src/runtime/compaction/grouping.rs:157-171` — brittle string-matching on lowercased content for classification with magic strings repeated between functions and duplicated in `mod.rs:85-88`.
- [ ] `src/runtime/compaction/summarize.rs:93` `let _ = decision;` — dead/unused parameter suggesting incomplete multi-strategy implementation.
- [ ] `src/runtime/compaction/artifact_store.rs:1-183` — every SQL error wrapped with a bespoke `.map_err` closure ~10 times — needs a shared `db_err(context)` helper.
- [ ] `src/runtime/compaction_store.rs:130-169,208-244` — manual `row.try_get(...)` field-by-field decode (~9 fields, done twice) instead of `sqlx::FromRow`/`query_as` used elsewhere in the same module family.
- [ ] `src/runtime/bearwire_projection/mod.rs:30-175` vs `wire.rs:168-346` — two large parallel matches transcode the same `RuntimeSemanticEvent` into two wire formats with duplicated error-category-to-string mapping — share one `RuntimeErrorCategory -> &str` function.
- [ ] `src/runtime/bearwire_projection/wire.rs:15-19` hand-rolled `impl Default for BearWireEventScope` instead of `#[derive(Default)]` with `#[default]` attribute.
- Overall: functionally clear but leans on `format!("{:?}", ...)` for persisted strings (4+ spots), duplicates small utilities 3+ times instead of sharing, one real `.expect()` smell in `role.rs`, and inconsistent SQL-decoding style within the same module family.

### den-runtime — agent_loop/{transcript,overflow_retry,tool_outcome,tool_policy,runtime_context,session_store}.rs, agent_assist/{agent_diagnostics,assistant_display,runtime_stream_parser,agent_summary,agent_prefill}.rs
- [ ] `src/agent_loop/session_store.rs:80,87,96,106` `.expect("agent loop session lock")` on every mutex access — will panic the whole process on lock poisoning; use `unwrap_or_else(|e| e.into_inner())` or a poison-tolerant wrapper.
- [ ] `src/agent_loop/transcript.rs:53-54,112-113` — silently swallows JSON parse errors for tool-call args via `unwrap_or_else`, duplicated verbatim in two places — extract `parse_tool_arguments`.
- [ ] `src/agent_loop/tool_outcome.rs:22-36,74-80` — "is this an error" detection via ad hoc string prefix/substring sniffing rather than a typed result envelope — brittle.
- [x] `src/agent_assist/agent_summary.rs:23-46` vs `agent_prefill.rs:15-38` — identical `pick_str`/`model_field` helpers duplicated verbatim between files (3rd copy also in `agent_diagnostics.rs:53-64`) — share via an `agent_assist` util module. **DONE** (duplication batch): added `agent_assist::json_fields` with shared `pick_str`/`model_field`; updated all three consumers; `den-runtime` clippy green.
- [ ] `src/agent_loop/transcript.rs:19-168` — `spawn_persist_native_agent_step`/`spawn_persist_web_chat_turn` duplicate the "approval_required → policy reason" ternary verbatim — extract `native_policy_reason(...)`.
- [ ] `src/agent_loop/transcript.rs` — four public `spawn_persist_*` functions each take 6-8 loosely-related positional args — use a `PersistenceIdentity` struct.
- [ ] `src/agent_assist/runtime_stream_parser.rs:56-235` `runtime_stream_event_from_provider_json` — ~180-line match with 10+ chained `.or_else()` field lookups per branch — genuine god-function, split per message-type.
- [ ] `src/agent_loop/runtime_context.rs:66-83` `assemble_den_owned_runtime_supplement` — two entirely unused parameters (`_client_context`, `_compaction_state`) in a public async fn.
- [ ] `src/agent_loop/overflow_retry.rs:69` `rebuild_messages_after_overflow_compaction` takes unused `_config: &Config`.
- [ ] `src/agent_assist/assistant_display.rs:70-83` `strip_prompt_scaffolding_prefix` hardcodes magic sentinel strings inline rather than named constants, inconsistent with `overflow_retry.rs:22`'s `COMPACTION_BLOCK_MARKER` const pattern.
- [ ] `src/agent_assist/agent_prefill.rs:54-58` — `while s.contains("--") { s = s.replace(...) }` O(n²)-ish manual collapse loop instead of a regex (already a dependency elsewhere in the crate).
- [ ] `src/agent_loop/session_store.rs:66` `AgentLoopSessionStore` derives `Default` but also hand-writes a redundant `new()`.
- Overall: generally tidy runtime/glue layer, but recurring copy-pasted small parsing helpers (`pick_str`, `model_field`, approval-reason ternaries) belong in one shared module; one genuine god-function (`runtime_stream_event_from_provider_json`); a couple of dead/unused parameters signal half-finished plumbing.

### den-runtime — turn_runs.rs, turn_steps.rs, turn_runner.rs, surface_projection.rs, tool_output_artifacts.rs, context_budget.rs, bearwire_events.rs, conversation_ids.rs, turn_ids.rs
- [x] `src/turn_runs.rs:143,169` vs `turn_steps.rs:79,153` — active-state SQL lists (`'streaming_model', 'waiting_for_client', ...`) hand-duplicated as raw string literals in multiple places instead of derived from the state enum's variants — drift risk. **DONE** (typing batch): centralized active run/step SQL state lists behind module constants used at every query site.
- [x] `src/turn_steps.rs:110-114,137-141` `transition_step`/`transition_active_steps_for_run` take `state: &str` and re-validate via `try_from_storage`, inconsistent with `turn_runs.rs`'s `transition_run` which takes a typed enum directly. **DONE** (typing batch): both functions now take `TurnStepState`; call sites in `den-runtime` and `den-bearwire` pass enum variants.
- [x] `src/turn_steps.rs` — `TurnStepState` lacks `Serialize`/`Deserialize` even though sibling `TurnRunState` derives both. **DONE** (typing batch): derived `Serialize`/`Deserialize` with `snake_case` serde naming.
- [ ] `src/turn_runner.rs:87-132` `materialize_runtime_conversation_if_needed` mixes 3 responsibilities (classification/creation/session upsert) with duplicated early-return branches.
- [ ] `src/surface_projection.rs:114-120` `bearwire_client_method_for_action` and `expected_responder_action` string constants matched untyped in multiple places — an enum would give compiler-checked exhaustiveness.
- [x] `src/tool_output_artifacts.rs:105,120-145` `sqlx::query_as::<_, (7-tuple)>` decoded positionally via `row.0`..`row.6` — use a `#[derive(sqlx::FromRow)]` struct instead.  **DONE** (structural batch A): converted to `#[derive(sqlx::FromRow)]` + `query_as` (pure decoding swap, SQL unchanged); verified clippy `--all-targets -D warnings` clean.
- [ ] `src/tool_output_artifacts.rs:16` `pub source: &'static str` on a public struct forces callers to only pass literals — unusual API choice, use `String` or an enum.
- [ ] `src/bearwire_events.rs:46` `format!("evt_{id}")` embeds an ID-prefixing convention inline rather than a typed constructor, inconsistent with `conversation_ids.rs`/`turn_ids.rs`'s dedicated ID newtypes.
- [ ] `src/turn_ids.rs:6-32` `string_id!` macro's `new()` returns `Option<Self>` (None on blank input) rather than `Result<Self, _>`, inconsistent with the crate's `DenError::ValidationError` convention used everywhere else.
- [ ] `src/turn_ids.rs:34-39` — five ID newtypes generated via macro, but most of the crate (`turn_runs.rs`, `turn_steps.rs`, `surface_projection.rs`) still passes raw `String`/`&str` for the same concepts — newtypes appear largely unused, suggesting an incomplete migration.
- Overall: consistent `DenError`+`?` usage and no stray unwrap/panic in non-test code, but organic duplication (hand-repeated SQL state lists, parallel `&str`-vs-enum APIs between sibling modules, a positional-tuple decode, an underused typed-ID module) point to incremental, un-refactored growth.

### tools/bear-armature — main.rs (lines 1-2165, 6000-9736; the god-file structure and `handle_request` dispatch were already flagged previously)
- [ ] `src/main.rs:1803-1955` `RuntimeConfig::from_env_and_args` — ~150-line function mixing arg parsing/env fallback/validation/process-exit side effects, hard to test in isolation.
- [ ] `src/main.rs:1655-1955` `BrowserBridgeConfig::from_args`/`RuntimeConfig::from_env_and_args` duplicate the same env-then-CLI-override pattern — share a helper.
- [ ] `src/main.rs:513-537` `config_value_from_params`/`mode_value_from_config_params` are byte-for-byte identical function bodies.
- [ ] `src/main.rs:1526-1531,1811-1820,1945-1952` — config validity checked via scattered string-matching + `eprintln!`+`process::exit` inside a "parse" constructor rather than returning a `Result`.
- [ ] `src/main.rs:1749-1875` `AcpConnectionArgs` — 9-field plain data bag with 3 near-duplicate literal constructions (~25 lines of repetition) — needs `Default`-based overrides.
- [ ] `src/main.rs:6438-6647` `handle_sse_frame` and downstream — marked `#[allow(dead_code)]` yet still fully maintained, duplicating logic now handled by `handle_den_event` — delete or document why kept.
- [ ] `src/main.rs:6828-7405` `handle_tool_request_event` — ~580-line function handling phase transitions/permission flow/dispatch/result posting inline; repeated `task_registry.set_phase(...); log_tool_task_phase(...)` pairs (9+ times) — extract `transition_phase(...)`.
- [ ] `src/main.rs:7195-7238` and `8856-9364` — tool dispatch is an if/else-if chain matching on tool-name strings, duplicated across 4-5 separate functions (`tool_display`, `tool_target_kind`, `tool_call_title`, `tool_supports_input_location`) — a `ToolKindRegistry`-style table keyed by tool name would remove this repeated pattern.
- [ ] `src/main.rs:8119-8664` `handle_permission_request_event` — one ~545-line function building display strings/permission bodies/handling 3 response shapes inline — split into per-tool-kind builders.
- [ ] `src/main.rs:8195-8317` — target/label/title derivation for permission requests repeats the same 4-branch if/else-if shape 3 times — extract a `PermissionPresentation` struct built once.
- [ ] `src/main.rs:8856-9100` `tool_display` — 250-line match over ~35 string literals returning pure data — use a static table instead.
- [ ] `src/main.rs:9366-9382` `friendly_tool_status` re-derives `ToolDisplay::from_event` every call even though most callers already computed it earlier — needless recomputation.
- [ ] `src/main.rs:9586-9588` `write_notification` constructs a brand-new `JsonRpcTransport::default()` per call instead of taking `&JsonRpcTransport` — silently ignores the "real" transport held in shared state; works only because the type is stateless-by-design, but undocumented.
- [ ] `src/main.rs:7625-7847` `post_local_tool_error_result`/`post_permission_result`/`post_tool_result`/`post_adapter_environment` — repeat the same `if bear_debug_verbose() { eprintln!(...) }` block — extract `log_bearwire_response(...)`.
- [ ] `src/main.rs:9616-9698` — error classification (`authenticate_json_rpc_error`, `looks_like_configuration_error`, `looks_like_den_connectivity_error`) matches on formatted message substrings (`"Missing DEN_TOKEN"`, `"HTTP 502"`) rather than typed error variants — fragile, breaks if messages are reworded.
- [ ] `src/main.rs` — dead code behind `#[allow(dead_code)]` at multiple sites (`SseFrameOutcome:171`, `LocalToolStatus` variants:313, `handle_sse_frame`:6438-6647, `with_adapter_contract`:9607, `den_compatibility_status_message`:9700) — worth a dedicated cleanup pass.
- Overall: functionally coherent but suffers from "one god function per concern" plus pervasive string-typed dispatch instead of enums/structured types; biggest win would be a `ToolKind`-keyed static table collapsing the parallel string-matching functions.
- **Coverage note:** this pass covered lines 1-2165 and 6000-9736; lines 2165-6000 (the `handle_request` dispatch match) were intentionally not re-read, per the already-flagged god-function finding from the first pass.

### den-runtime — reflection/{mod,archive_harvest}.rs, runtime/compaction/{overflow,policy,render}.rs, runtime/compaction_observability.rs, runtime/pair_turn.rs, runtime/role_registry.rs, runtime/provider/mod.rs
- [ ] `src/reflection/archive_harvest.rs:57,118` — `let _ = record_harvest_mark(...).await?` — redundant `let _ =` since `?` already propagates.
- [ ] `src/reflection/archive_harvest.rs:169,188,193` — all DB/decode failures collapse into `DenError::Database(String)` via ad-hoc `format!`, losing error source/type info.
- [ ] `src/reflection/archive_harvest.rs:191-193` `decode_summary` clones the whole `Value` just to call `from_value` — accept owned `Value` to skip the clone.
- [ ] `src/reflection/mod.rs:1-6` — module doc mentions "conductor loop" but not `archive_harvest`, which is also declared here — stale doc.
- [ ] `src/runtime/compaction/overflow.rs:31-35` `den_error_indicates_context_overflow` only special-cases 3 `DenError` variants with a silent catch-all `_ => false` — easy to miss when new variants carrying the same message are added.
- [ ] `src/runtime/compaction/policy.rs:17-23,36-41` `CompactionMode::parse`/`CompactionTiming::parse` are hand-rolled parsers duplicating `impl FromStr` — use `FromStr` for idiomatic `.parse()` support.
- [ ] `src/runtime/compaction/policy.rs:17-23` — unrecognized `COMPACTION_MODE` values silently become `Observe` rather than erroring/logging — could mask config typos.
- [ ] `src/runtime/compaction/policy.rs:45-72` `compaction_policy_for_profile` — 4 near-identical struct literals differing only in 3 numeric fields — use a small lookup table.
- [ ] `src/runtime/compaction/render.rs:6-42` — six repetitive `if !summary.X.is_empty() { sections.push(...) }` blocks — data-driven loop would cut ~35 lines.
- [ ] `src/runtime/compaction/render.rs:44-49` — `sections.len() == 1` used as a fragile implicit "nothing rendered" check.
- [ ] `src/runtime/compaction_observability.rs:29-65` `build_compaction_applied_event`/`build_compaction_skipped_event` hand-assemble the same 8-field struct — unify via `RuntimeCompactionEvent::skipped(...)`/`::applied(...)` associated functions.
- [ ] `src/runtime/role_registry.rs:17` `new(pool, _config)` accepts and discards `_config` entirely — drop the parameter or it misleads callers.
- [ ] `src/runtime/role_registry.rs:26-29` — intentional query duplication (documented, to avoid a crate cycle) has no test/lint guarding the two copies stay in sync.
- [ ] `src/runtime/role_registry.rs:41-44` — synthetic binding-id convention (`format!("den-native:{bear_id}:...")`) isn't documented as part of `resolve_binding`'s public contract.
- [ ] `src/runtime/provider/mod.rs:1-12` — pure re-export shim with zero documentation on why this namespace exists separately from `den_protocol`.
- Overall: generally tidy and idiomatic (consistent `?` propagation, plain-data structs), but recurring String-typed errors swallowing causes and several near-duplicate struct-construction functions that could collapse via constructors/tables.

### den-runtime — bears/{model,templates,runtime_plan,sync}.rs, agent_loop/{mod,pending_tools,approvals,strategy,policy}.rs
- [ ] `src/bears/model.rs:63` `parsed_profile` returns `Result<BearProfile, String>` — a typed parse error would be more idiomatic.
- [ ] `src/bears/runtime_plan.rs:20` `effective_runtime_plan` — hand-rolled, only one-level-deep JSON merge with no documentation that only `memory` is deep-merged (other top-level keys are fully overwritten) — surprising and undocumented behavior.
- [x] `src/bears/sync.rs:24,31,38` — sync status compared via literal `&str` (`"failed"`/`"skipped_missing_binding"`/`"synced"`) in 3 methods — use a `SyncStatus` enum with `Display`/`Serialize`. **DONE** (typing batch): added `BearProfileSyncStatus` with stable snake_case serialization; outcome construction and summary filters now use enum variants; `den-service` clippy green.
- [x] `src/bears/sync.rs:21,28` `failed_profiles`/`skipped_profiles` allocate a `Vec` per call even when immediately consumed via `.into_iter()` at the only call site — could return `impl Iterator`. **DONE** (duplication/perf batch): both methods now return iterators; `diagnostic_message` uses `peekable()` to avoid allocation; `den-service` clippy green.
- [ ] `src/agent_loop/mod.rs:22-66` — barrel file re-exporting ~45 items across 13 submodules with no doc explaining the grouping; overlapping module names (`pending_tools`, `tool_policy`, `policy`) risk confusion for new readers.
- [ ] `src/agent_loop/pending_tools.rs:10` `open.clone_from(calls)` reclones the entire tool-call vector on every assistant turn in history instead of tracking the index of the last one.
- [ ] `src/agent_loop/approvals.rs:9-14` `NativeApprovalRow.status: String` — same stringly-typed status pattern as `sync.rs`, no shared enum despite the decision itself already being typed elsewhere.
- [ ] `src/agent_loop/approvals.rs:49,75` — duplicated `.map_err(|e| DenError::System(format!(...)))` wrapping pattern within one file.
- [ ] `src/agent_loop/policy.rs:7-9` `StrategyPolicyInput` uses raw `&'static str` for `task_kind`/`difficulty` matched via `matches!(..., Some("investigation"))` — a proper `enum TaskKind`/`enum Difficulty` would prevent typos and enable exhaustiveness checking.
- [ ] `src/agent_loop/strategy.rs:7` `fanout_n: u8` has no doc on valid range/meaning (0 = disabled?).
- [ ] `src/agent_loop/{policy,strategy,approvals}.rs` — no `#[cfg(test)]` coverage, unlike sibling files (`model.rs`, `runtime_plan.rs`, `pending_tools.rs`) which all have tests.
- Overall: small, well-scoped, largely idiomatic files — main recurring theme is stringly-typed status/kind fields that should be small enums, plus missing test coverage in the policy/strategy/approvals trio; no unwrap/panic/unsafe misuse found.

### den-runtime — native_runtime/{mod,legacy_memory_tools,tool_invoker,profile,profile_briefing,openai_stream,tools}.rs, llm/mod.rs, memory/{mod,bear_observations}.rs, recall/{mod,chunking,policy,registry}.rs, conversation/{message_types,persistence}.rs
- [ ] `src/conversation/message_types.rs`, `persistence.rs` — bare re-export files; unclear why these thin wrappers exist in `den-runtime` at all vs. depending on `den_service` directly — dead indirection.
- [ ] `src/native_runtime/openai_stream.rs:18-193` `responses_byte_stream_to_event_stream`/`openai_byte_stream_to_event_stream_with_telemetry` — near-identical ~90-line `poll_fn` state machines duplicated wholesale — extract a shared generic driver.
- [ ] `src/native_runtime/openai_stream.rs:44-56,130-142,158-169` — same terminal-event `matches!` check copy-pasted 3 times — extract `is_terminal_or_pause(&event)`.
- [ ] `src/recall/registry.rs:38,68,90,139,165,194` — six near-identical sqlx-error-wrapping lines — extract a shared `wrap(op)` helper.
- [ ] `src/recall/registry.rs:101` `#[allow(clippy::too_many_arguments)]` on `upsert_passage` (9 params) — use a `NewPassage<'a>` struct (matching the `CreateBearObservation` convention already used in `bear_observations.rs`).
- [x] `src/recall/registry.rs:42-47,167,196` — manual `Row::get` field mapping instead of `sqlx::FromRow` on `ExistingPassage`.  **DONE** (structural batch A): converted to `#[derive(sqlx::FromRow)]` + `query_as` (pure decoding swap, SQL unchanged); verified clippy `--all-targets -D warnings` clean.
- [x] `src/memory/bear_observations.rs:109-124` `row_from_sql` manually maps every column instead of deriving `sqlx::FromRow`.  **DONE** (structural batch A): converted to `#[derive(sqlx::FromRow)]` + `query_as` (pure decoding swap, SQL unchanged); verified clippy `--all-targets -D warnings` clean.
- [ ] `src/memory/bear_observations.rs:44-51,77-79,96-98` — same 12-column SELECT/RETURNING list repeated verbatim 3 times — extract a `const OBSERVATION_COLUMNS`.
- [ ] `src/native_runtime/tools.rs:27` `den_tools_for_profile(_config, role)` — unused `_config` parameter.
- [ ] `src/native_runtime/tools.rs:57-59` `&first_sentence[..96]` byte-slices a `&str` from arbitrary tool descriptions — will panic on non-ASCII char boundary.
- [ ] `src/native_runtime/tools.rs:74-125` — keyword-matching heuristics (`lower.contains(keyword)`) for tool-surface gating will misfire on substrings.
- [ ] `src/native_runtime/profile.rs:18-141` `NativeCapabilityProfile::for_profile` — ~120-line match with heavily duplicated nested struct-literal shapes across profile variants — use a builder or per-field table.
- [ ] `src/native_runtime/profile.rs:92-136` — nested `if profile == BearProfile::Work` inside an already-destructured `Chat | Work` match arm — confusing double-dispatch, split into separate arms.
- [ ] `src/native_runtime/profile_briefing.rs:9-26` — builds prompt via repeated `push_str(&format!(...))`, allocating a new String per item — use `write!(prompt, ...)`.
- [ ] `src/recall/chunking.rs:28-30` — manual hex-encoding loop instead of a `hex`/`data-encoding` crate.
- [ ] `src/recall/chunking.rs:69-70` — allocates twice (`collect()` then `trim().to_string()`) where trimming char-slice bounds first would allocate once.
- [ ] `src/recall/policy.rs:17-20` `EXCLUDED_KINDS`/`PROFILE_LOCAL_INDEXABLE_KINDS` are untyped `&[&str]` — an enum for memory `kind` would give compile-time exhaustiveness.
- [ ] `src/native_runtime/legacy_memory_tools.rs:13-16` `filter_client_tools_for_native_runtime` clones a potentially large `Value` unnecessarily in the common non-array case — return `Option<&Value>` instead.
- [ ] `src/native_runtime/tool_invoker.rs:33,43-45` `set_tool_invoker` silently discards `OnceLock::set` error — a double-init bug would be swallowed with no log/assert.
- Overall: generally solid — proper `DenError`+`?` propagation, no unwrap/panic in library paths, thorough module docs — but real duplication (SSE state machines, terminal-event matches, sqlx error wrapping, repeated column lists) that should be factored into shared helpers, plus a couple of byte-slicing panics on non-ASCII input.

### den-web — admin/users.rs, onboarding.rs, bear/profile.rs, user/settings/mod.rs, status.rs, bear/create_support.rs
- [ ] `src/admin/users.rs:274,298,330` — repeated `.ok_or_else(|| CustomError::NotFound(...))` boilerplate 3x — extract a helper.
- [ ] `src/admin/users.rs:385-392` — invite code generation reimplements a random alphanumeric string inline instead of `rand::distributions::Alphanumeric`; `use rand::Rng` imported mid-function rather than at file top.
- [ ] `src/admin/users.rs:339-350` — raw SQL INSERT written inline in a handler rather than delegated to `user_db`/`email` module, breaking module boundary.
- [ ] `src/onboarding.rs:196-205` `first_bear_post` — ~120-line function doing form validation, slug validation, model validation, DB insert, key provisioning, membership grant, and native provisioning all inline — extract named steps.
- [ ] `src/onboarding.rs:264-277` — `let _ = bears_db::delete_bear(...)` on a rollback path discards the cleanup error silently with no `tracing::warn!`.
- [ ] `src/bear/profile.rs:48-55` `BearRoleViewRow::from_agent` takes an unused `_role: BearProfile` parameter.
- [ ] `src/bear/profile.rs:183` `agent.clone()` unnecessary since `agent` isn't reused after.
- [ ] `src/bear/profile.rs:57-171` — five near-identical `match role { ... }` functions (label/description/plain_name/surfaces/capabilities/memory_rules) — consolidate into a single data table as `BearProfile` metadata.
- [x] `src/user/settings/mod.rs:85,119` — two `.unwrap()` calls in request handlers inconsistent with the rest of the crate's `ok_or_else` convention — should be `.ok_or_else(|| CustomError::NotFound(...))?`. **DONE** (safety batch): both handlers now return `CustomError::NotFound`; `den-web` clippy green.
- [ ] `src/status.rs:123-144` `build_deploy_rows` returns a `Vec` with always exactly one element — unnecessary ceremony, name implies multi-row generality it doesn't have.
- [ ] `src/bear/create_support.rs:141-157,360-377,389-408` — `admin_bear_new_form_context`/`bear_new_form_context`/`admin_bear_edit_page_context` near-identical bodies — unify into one parameterized helper.
- [ ] `src/bear/create_support.rs:494-508` — three sequential empty-checks for chat/pair/watch roles but not curate/work — asymmetric, confusing (not a correctness call, but a style/coverage gap).
- [ ] `src/bear/create_support.rs:52-137` — four `impl From<&Bear> for XxxForm` blocks all clone every string field — repetitive, candidate for a macro/trait helper.
- Overall: generally consistent `CustomError`+`?` usage, but two stray `.unwrap()`s in `settings/mod.rs` break that consistency, and `create_support.rs`/`onboarding.rs` show real duplication that would benefit from extraction.

### den-service — bears/managed_blocks.rs, client_sessions.rs, bears/prompt_fragments/*, recall/qdrant.rs, model_selection.rs, archived_conversations.rs
- [x] `src/bears/managed_blocks.rs:15-27` `ManagedBlockResolutionRow` — 11-tuple type alias, extremely poor readability at the destructuring site (427-439, named manually) — use `#[derive(FromRow)]`.  **DONE** (structural batch A): converted to `#[derive(sqlx::FromRow)]` + `query_as` (pure decoding swap, SQL unchanged); verified clippy `--all-targets -D warnings` clean.
- [x] `src/client_sessions.rs:195-217` `client_session_row_from_sql` manually maps every column by string key instead of `#[derive(FromRow)]`+`query_as`, unlike other files in the same batch (`managed_blocks.rs`, `archived_conversations.rs`) — inconsistent with crate convention.  **DONE** (structural batch A): converted to `#[derive(sqlx::FromRow)]` + `query_as` (pure decoding swap, SQL unchanged); verified clippy `--all-targets -D warnings` clean.
- [ ] `src/client_sessions.rs:60-105` `trusted_workspace_context` — ~45-line chain of `.as_ref().and_then().and_then().map()...` — extract named helpers (`extract_roots`, `extract_cwd`).
- [ ] `src/bears/prompt_fragments/render.rs:45-46,60-61` `render_turn_text`/`render_compile_time_text` construct a brand-new `minijinja::Environment` per call — cache/share if this is a hot path.
- [ ] `src/bears/prompt_fragments/bundle.rs:26` — `DenError::Parsing` used here vs `DenError::ValidationError` used for the same "malformed input" family elsewhere (`frontmatter.rs`) — inconsistent error-variant choice.
- [ ] `src/bears/prompt_fragments/registry.rs`, `bundle.rs` — both `PromptFragmentRegistry`/`PromptBundleRegistry` duplicate an identical "insert or error on duplicate key" pattern — share a generic `insert_unique` helper.
- [ ] `src/model_selection.rs:226-239` `resolve_model_option` — silently falls through to the static registry on any DB error (not just "not found"), masking real failures as fallback-worthy.
- [ ] `src/model_selection.rs:80-90` `simplify_model_option_label_for_acp` — dense string-processing chain with no comment explaining expected label formats.
- `src/recall/qdrant.rs`, `src/archived_conversations.rs`, `tools/bear-armature/build.rs` — all clean/idiomatic, no significant findings.
- Overall: solid error handling with `DenError` throughout and good doc comments in most files; main issues are structural (tuple-based row type, manual row mapping) rather than error-handling misuse.

## Remaining coverage gaps (not yet audited)
- `den-runtime` — 62 of its ~100 files reviewed (crate is 33.7k lines, the largest in the workspace). Everything over ~50 lines has now been read. Genuinely unreviewed: `llm/mod.rs` (21 lines, trivial re-exports), a handful of tiny mod.rs re-export shims, and `*_tests.rs` files (out of scope for production-code review).
- `tools/bear-armature` — only `src/main.rs` lines 2165-6000 remain unread in depth (the already-flagged `handle_request` dispatch match itself — its god-function nature is already documented, re-reading it line-by-line would add volume, not new signal).
- `den-web`, `den-service` — no known gaps remain; all files identified in the initial survey have been reviewed.

## Audit complete
This audit has now covered essentially the entire workspace: every file over ~50 lines across `den-protocol`, `den-api`, `den-http`, `den-llm`, `den-docket`, `den-oauth`, `den-bearwire`, `den-memory`, `den-core`, `den-service`, `den-web`, `services/den/src`, `den-runtime`, and `tools/bear-armature`. The only intentionally-unread code is `bear-armature`'s `handle_request` dispatch match (already flagged as a god-function, so reading it fully would only add more instances of the same finding) and trivial test/re-export files. Further passes would mostly re-confirm the eight cross-cutting themes below rather than surface new categories of issue — this is a good stopping point for coverage; the remaining work is fixing, not finding.

## Cross-cutting themes observed across the whole workspace
1. **God-files/god-functions are the single most common issue** — nearly every crate has at least one 200+ line function or 1000+ line file mixing unrelated concerns (`den-docket::model.rs`/`db.rs`, `den-runtime::assembler.rs`/`key_memory_projection.rs`, `den-oauth::endpoints.rs::token_post`, `den-bearwire::run.rs`/`client.rs`, `den-web::bear/settings.rs`, `den-core::descriptor/mod.rs`/`client_tools.rs`, `bear-armature::main.rs`/`fs.rs`).
2. **Error handling is inconsistent workspace-wide** — some modules use `thiserror` well (`den-http::auth_backend`, `startup.rs`, `den-core::DenError`), others hand-roll the same boilerplate (`den-http::errors::CustomError`) or funnel everything into a stringly-typed catch-all (`DenError::System(String)` in `den-llm`, `den-memory`, `den-service`).
3. **Stringly-typed domain concepts instead of enums** recur constantly — status/reason/kind fields represented as `&str`/`String` with hand-matched literals (`den-docket`'s tagged-union refs, `den-bearwire`'s reason tuples, `bear-armature`'s risk/mode strings, 10+ hand-written `as_str()` impls in `den-service` that should be `Display`).
4. **Duplicated logic instead of extraction** — near-identical function pairs/triples show up in almost every crate (retry loops in `den-llm`, wrapper pairs in `den-runtime`'s obligations subsystem, SSE response builders in `den-web`, projection builders in `den-service`, parallel tool-registry tables in `den-core`).
5. **A handful of reachable panics in production code paths** — `den-http::email::mailgun_client` `.expect()`, `den-web::observability::chat_proxy_stream.rs:473` `.expect()`, `den-web::user/account/mod.rs:290` unwrap-on-Option, `den-runtime::agent_loop/session_store.rs` `.expect()` on every mutex lock, `den-runtime::runtime/role.rs:224` `.expect()` on a required registry.
6. **Manual reimplementation of well-solved problems** — glob matching and unified-diff parsing in `bear-armature`, git plumbing via subprocess instead of `git2`, hand-rolled CSRF tokens in `den-oauth`, hand-rolled UUID sniffing in `den-runtime`, hand-rolled SSE frame parsing in `bear-armature::bearwire.rs` instead of an `eventsource-stream`-style crate.
7. **`format!("{:?}", ...)` (Debug) used to build persisted/wire strings** — seen 4+ times across `den-runtime`'s compaction subsystem alone (`mod.rs`, `artifact_store.rs`, `compaction_store.rs`) — fragile since Debug output isn't a stable contract; should be explicit `Display`/`as_str()`.
8. **Typed newtypes introduced but not adopted** — `den-runtime::turn_ids.rs` defines 5 ID newtypes via macro, but most of the crate still passes raw `String`/`&str` for the same concepts, suggesting an incomplete migration rather than a design gap.

Status: audit is thorough but not exhaustive — roughly two-thirds of the workspace by line count has been read in depth (all of den-protocol/api/http/llm/docket/oauth/core/memory/bearwire, most of den-web/den-service, ~1/3 of den-runtime, spot-checks of bear-armature). The findings above are actionable as-is; resuming coverage of den-runtime and bear-armature would be the highest-value next step given their size and the god-file density already observed there.
