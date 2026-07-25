-- Work runs are job dispatch attempts, not task dispatch attempts. Task
-- progress remains represented by bear_task_run_state and bear_task_events.
--
-- Old versions permitted one active run per task, so an existing job can have
-- several active rows. Keep the most advanced run (then the oldest queued) as
-- the job's continuing dispatch attempt and cancel the superseded attempts
-- before enforcing the one-active-run-per-job invariant.
DROP INDEX IF EXISTS idx_bear_work_runs_one_active_per_task;

WITH ranked_active_runs AS (
    SELECT
        id,
        row_number() OVER (
            PARTITION BY job_id
            ORDER BY
                CASE state
                    WHEN 'running' THEN 5
                    WHEN 'reporting' THEN 4
                    WHEN 'provisioning' THEN 3
                    WHEN 'claimed' THEN 2
                    WHEN 'queued' THEN 1
                END DESC,
                queued_at ASC,
                id ASC
        ) AS position
    FROM bear_work_runs
    WHERE state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting')
)
UPDATE bear_work_runs AS work_run
SET
    state = 'cancelled',
    cancel_requested = TRUE,
    finished_at = COALESCE(finished_at, now()),
    updated_at = now(),
    error = COALESCE(error, 'Superseded while consolidating task-scoped work runs into one job-scoped run.')
FROM ranked_active_runs AS ranked
WHERE work_run.id = ranked.id
  AND ranked.position > 1;

ALTER TABLE bear_work_runs DROP COLUMN IF EXISTS task_id;
CREATE UNIQUE INDEX IF NOT EXISTS idx_bear_work_runs_one_active_per_job
    ON bear_work_runs (job_id)
    WHERE state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting');
