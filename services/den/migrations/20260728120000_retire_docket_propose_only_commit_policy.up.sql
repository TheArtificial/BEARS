-- Retire the obsolete `propose_only` Docket commit policy. Branch-backed
-- publishing policies (`per_task` / `per_job`) are the review boundary for
-- source-changing work; `none` remains for jobs with no source changes
-- expected.

UPDATE bear_jobs
SET commit_policy = 'none', updated_at = now()
WHERE commit_policy = 'propose_only';

ALTER TABLE bear_jobs
    DROP CONSTRAINT IF EXISTS bear_jobs_commit_policy_check;

ALTER TABLE bear_jobs
    ADD CONSTRAINT bear_jobs_commit_policy_check
    CHECK (commit_policy IS NULL OR commit_policy IN ('none', 'per_task', 'per_job'));
