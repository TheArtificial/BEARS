#!/usr/bin/env bash
# End-to-end acceptance walkthrough for the native `work` sandbox flow:
# a Docket job on a remote-git work surface, executed in a sandbox by a
# headless armature, with the result pushed to the upstream work branch.
#
# Requirements (not a CI test — run on a docker host with a live stack):
#   - docker + the sandbox images built (scripts/build-sandbox-image.sh)
#   - a running Den (RUN_API + RUN_WORKERS) with SANDBOX_SERVER_URL pointed
#     at the provider this script starts, and a working LLM substrate
#   - psql access to the Den database (DATABASE_URL)
#   - BEAR_ID: the bear to run the job as; USER_ID: the requesting user
#
# What it does:
#   1. Creates a bare git repo in $E2E_DIR/upstream.git seeded with NOTES.md —
#      this is the "remote upstream" work surface.
#   2. Writes a roots file exposing it as root `e2e` with the base image.
#   3. Starts the sandbox provider (RUN_SANDBOX) against that roots file.
#   4. Seeds a Docket job (commit_policy per_task) with one work task:
#      append a line to NOTES.md.
#   5. Enqueues a work run and waits for the dispatch worker to finish it.
#   6. Asserts the upstream gained a den/job-* branch whose tip changes NOTES.md.
set -euo pipefail

: "${DATABASE_URL:?set DATABASE_URL (Den database)}"
: "${BEAR_ID:?set BEAR_ID (uuid of the bear to run as)}"
: "${USER_ID:?set USER_ID (id of the requesting user)}"
E2E_DIR="${E2E_DIR:-$(mktemp -d /tmp/work-e2e.XXXXXX)}"
SANDBOX_PORT="${SANDBOX_PORT:-3202}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "== e2e dir: $E2E_DIR"

# 1. Bare upstream with an initial commit.
git init --bare -b main "$E2E_DIR/upstream.git"
git init -b main "$E2E_DIR/seed"
(
    cd "$E2E_DIR/seed"
    git -c user.name=e2e -c user.email=e2e@test.invalid commit --allow-empty -m init >/dev/null
    echo "# Notes" > NOTES.md
    git add NOTES.md
    git -c user.name=e2e -c user.email=e2e@test.invalid commit -m "seed NOTES.md" >/dev/null
    git push -q "$E2E_DIR/upstream.git" main
)

# 2. Roots file: the upstream as root `e2e`, base image catalog.
cat > "$E2E_DIR/roots.json" <<ROOTS
{
  "images": [ {"name": "base", "image": "${SANDBOX_IMAGE:-bears/sandbox:latest}", "default": true} ],
  "roots": [ {"name": "e2e", "upstream": {"url": "$E2E_DIR/upstream.git", "default_ref": "main"}} ]
}
ROOTS

# 3. Sandbox provider (backgrounded; killed on exit).
(
    cd "$REPO_ROOT/services/den"
    RUN_SANDBOX=true RUN_WEB=false RUN_API=false RUN_WORKERS=false \
    SANDBOX_PORT="$SANDBOX_PORT" \
    SANDBOX_ROOTS_CONFIG="$E2E_DIR/roots.json" \
    SANDBOX_WORKSPACES_DIR="$E2E_DIR/workspaces" \
    SANDBOX_SERVICE_TOKEN="${SANDBOX_SERVICE_TOKEN:-e2e-token}" \
    cargo run --quiet -- serve
) &
PROVIDER_PID=$!
trap 'kill $PROVIDER_PID 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
    curl -fsS "http://127.0.0.1:$SANDBOX_PORT/sandbox/v1/health" >/dev/null 2>&1 && break
    sleep 1
done
curl -fsS "http://127.0.0.1:$SANDBOX_PORT/sandbox/v1/health" >/dev/null
echo "== provider is up on :$SANDBOX_PORT"
echo "== ensure the Den worker process has:"
echo "   SANDBOX_SERVER_URL=http://<this-host>:$SANDBOX_PORT SANDBOX_SERVER_TOKEN=${SANDBOX_SERVICE_TOKEN:-e2e-token}"

# 4. Seed the Docket job + work task + queued run.
JOB_ID=$(psql "$DATABASE_URL" -qtA <<SQL
WITH job AS (
    INSERT INTO bear_jobs (bear_id, created_by_user_id, created_by_role, goal,
                           work_surface_ref, commit_policy, status, visibility)
    VALUES ('$BEAR_ID', $USER_ID, 'ui', 'work-e2e: append a line to NOTES.md',
            'e2e', 'per_task', 'ready', 'same_user')
    RETURNING id
), run AS (
    INSERT INTO bear_job_runs (job_id, trigger, state)
    SELECT id, 'manual', 'running' FROM job RETURNING id, job_id
), upd AS (
    UPDATE bear_jobs SET current_run_id = run.id FROM run WHERE bear_jobs.id = run.job_id
), task AS (
    INSERT INTO bear_tasks (bear_id, job_id, sibling_order, kind, scope, title, body,
                            completion_criteria, assigned_to_role)
    SELECT '$BEAR_ID', id, 0, 'execution', 'template',
           'Append an e2e marker line to NOTES.md',
           'Append the exact line "e2e marker" to the end of NOTES.md and commit the change.',
           '["NOTES.md ends with a line reading e2e marker", "the change is committed"]'::jsonb,
           'work'
    FROM job RETURNING id, job_id
)
INSERT INTO bear_work_runs (bear_id, job_id, task_id, job_run_id, root_name)
SELECT '$BEAR_ID', task.job_id, task.id, run.id, 'e2e' FROM task, run
RETURNING job_id;
SQL
)
echo "== seeded job $JOB_ID; waiting for the dispatch worker"

# 5. Wait for a terminal run.
for _ in $(seq 1 120); do
    STATE=$(psql "$DATABASE_URL" -qtA -c \
        "SELECT state FROM bear_work_runs WHERE job_id = '$JOB_ID' ORDER BY queued_at DESC LIMIT 1")
    echo "   run state: $STATE"
    case "$STATE" in
        succeeded) break ;;
        blocked|failed|cancelled|timed_out)
            psql "$DATABASE_URL" -c \
                "SELECT state, result_summary, error FROM bear_work_runs WHERE job_id = '$JOB_ID'"
            echo "E2E FAILED: run ended $STATE" >&2; exit 1 ;;
    esac
    sleep 5
done
[ "$STATE" = "succeeded" ] || { echo "E2E FAILED: run never finished" >&2; exit 1; }

# 6. The upstream must have the job's work branch with the change.
BRANCH=$(psql "$DATABASE_URL" -qtA -c "SELECT work_branch FROM bear_jobs WHERE id = '$JOB_ID'")
echo "== job work branch: $BRANCH"
git -C "$E2E_DIR/upstream.git" rev-parse "refs/heads/$BRANCH" >/dev/null
git clone -q --branch "$BRANCH" "$E2E_DIR/upstream.git" "$E2E_DIR/verify"
grep -q "e2e marker" "$E2E_DIR/verify/NOTES.md"
echo "== E2E OK: upstream $BRANCH carries the committed change"
psql "$DATABASE_URL" -c \
    "SELECT state, result_summary, result_refs->'published' AS published
     FROM bear_work_runs WHERE job_id = '$JOB_ID'"
