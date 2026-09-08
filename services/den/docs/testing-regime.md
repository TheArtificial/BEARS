# Den testing regime

Den uses layered checks. A passing build, lint, or readiness probe is not a substitute for a test that exercises the behavior changed.

## Local development

Run the smallest relevant checks first. Host-side compile-only checks normally use the checked-in SQLx metadata:

```bash
cd services/den
cargo fmt --check
SQLX_OFFLINE=true cargo check -p <touched-crate>
SQLX_OFFLINE=true cargo test -p <touched-crate> <focused-test-name>
```

A command that reports `0 tests` compiled a target but **did not validate the intended behavior**. Correct the test target, filter, features, or environment before claiming coverage.

For release or deployment-impacting changes, also build the production image:

```bash
cd services/den
docker build .
```

## Test layers

### Unit tests

Unit tests run in-process and should cover deterministic behavior, including ownership cancellation and transcript-repair logic. They should not require a live PostgreSQL instance.

Run the full unit layer with:

```bash
cd services/den
SQLX_OFFLINE=true cargo test --workspace --lib
```

### Postgres integration tests

Tests using `#[sqlx::test]` require a live Postgres server and create isolated migrated databases. They verify durable control-plane transitions that unit tests cannot prove.

Run them with a `DATABASE_URL` pointing to a disposable local or CI database:

```bash
cd services/den
DATABASE_URL=postgres://postgres:postgres@localhost:5432/den_test \
  cargo test -p den-bearwire
```

`den-bearwire` keeps these SQLx tests in its library test target. Use the ordinary package command above (or an explicit `--lib` focused command); do not use an invocation that discovers no tests. Confirm discovery first when changing targets:

```bash
cargo test -p den-bearwire -- --list
```

The Pair/Docket lifecycle suite must cover both terminal paths:

1. assign a Docket task to a Pair session without taking chat control;
2. focus it and verify chat control hands to a Docket run using that assigned task;
3. add a subtask while Docket control is active;
4. settle tasks with default optional settlement fields omitted;
5. verify all-settled completion ends Docket control and permits a fresh chat turn;
6. separately verify a blocked task ends Docket control, leaves the job recoverable, and permits a fresh chat turn.

Keep this lane independent from shared developer databases: pool exhaustion or contention must fail clearly rather than masquerade as a product regression.

### SQLx metadata and linting

The `Den – clippy gate` workflow runs on Den pull requests and relevant pushes. It verifies migration/query metadata freshness and runs strict workspace Clippy. It is a compile-and-lint gate, not behavior coverage.

### Container build

The image workflow builds and publishes deployable images on the deployment branches. It verifies the production Docker build and target platform, but does not replace unit or integration testing.

### Deployment checks

Deployment readiness and Git-SHA checks verify that the intended image started. Test-environment smoke coverage should exercise a disposable Pair/Docket focus-to-terminal-control cycle without calling an LLM. Never run mutating smoke tests against production.

## CI policy

For changes under runtime, BearWire, Docket, or migrations, CI should require:

1. SQLx metadata freshness and strict Clippy;
2. workspace unit tests;
3. a Postgres-backed BearWire/Docket integration lane;
4. a production image build before a deployment-impacting release is considered complete.

The Postgres integration lane uses a service-container database. It must run the actual `#[sqlx::test]` target; verify that the selected target and filter discover tests before labeling it a pass.

## Definition of done for Pair/Docket lifecycle changes

- Focused unit regressions pass.
- The live-Postgres lifecycle tests execute and pass for both completed and blocked termination.
- No test command intended as evidence reports zero discovered tests.
- SQLx metadata and Clippy pass.
- Docker build passes for release/deployment-impacting changes, or its omission is explicitly recorded.
- A test-deployment smoke test confirms focus control is released after a terminal run.
