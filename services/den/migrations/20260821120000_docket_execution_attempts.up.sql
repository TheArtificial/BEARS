-- Canonical task-level continuation authority. This is distinct from legacy
-- docket_execution_sessions and per-turn routing claims.
CREATE TABLE docket_execution_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('pair', 'work')),
    pair_session_id TEXT NULL,
    pair_run_id UUID NULL,
    work_run_id UUID NULL REFERENCES bear_work_runs (id) ON DELETE CASCADE,
    fence_epoch BIGINT NOT NULL CHECK (fence_epoch > 0),
    authorization_key UUID NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'authorized', 'running', 'paused', 'awaiting_user', 'stopping', 'settled', 'released'
    )),
    started_at TIMESTAMPTZ NULL,
    paused_at TIMESTAMPTZ NULL,
    settled_at TIMESTAMPTZ NULL,
    released_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (
        (owner_kind = 'pair' AND pair_session_id IS NOT NULL AND btrim(pair_session_id) <> ''
         AND pair_run_id IS NOT NULL AND work_run_id IS NULL)
        OR
        (owner_kind = 'work' AND work_run_id IS NOT NULL
         AND pair_session_id IS NULL AND pair_run_id IS NULL)
    ),
    CHECK (state <> 'running' OR started_at IS NOT NULL),
    CHECK (state NOT IN ('paused', 'awaiting_user') OR paused_at IS NOT NULL),
    CHECK (state <> 'settled' OR settled_at IS NOT NULL),
    CHECK (state <> 'released' OR released_at IS NOT NULL)
);

CREATE UNIQUE INDEX docket_execution_attempts_authorization_key_idx
    ON docket_execution_attempts (authorization_key);
CREATE UNIQUE INDEX docket_execution_attempts_live_task_idx
    ON docket_execution_attempts (task_id)
    WHERE state IN ('authorized', 'running', 'paused', 'awaiting_user', 'stopping');
CREATE UNIQUE INDEX docket_execution_attempts_live_pair_owner_idx
    ON docket_execution_attempts (pair_session_id)
    WHERE state IN ('authorized', 'running', 'paused', 'awaiting_user', 'stopping')
      AND owner_kind = 'pair';
CREATE UNIQUE INDEX docket_execution_attempts_live_work_owner_idx
    ON docket_execution_attempts (work_run_id)
    WHERE state IN ('authorized', 'running', 'paused', 'awaiting_user', 'stopping')
      AND owner_kind = 'work';

CREATE INDEX docket_execution_attempts_task_fence_idx
    ON docket_execution_attempts (task_id, fence_epoch DESC);

COMMENT ON TABLE docket_execution_attempts IS
    'Canonical Docket-owned task-level continuation authority for Pair and Work.';
