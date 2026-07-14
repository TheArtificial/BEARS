-- Conversation-linked Docket objectives.
--
-- A task-oriented conversation gets one mutable Docket-backed objective. The
-- objective reuses the existing bear_jobs/bear_tasks machinery; top-level
-- tasks under the job are the apparent projects in that conversation.

ALTER TABLE bear_jobs
    ADD COLUMN IF NOT EXISTS source_conversation_id TEXT NULL,
    ADD COLUMN IF NOT EXISTS objective_kind TEXT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'bear_jobs_objective_kind_check'
    ) THEN
        ALTER TABLE bear_jobs
            ADD CONSTRAINT bear_jobs_objective_kind_check
            CHECK (objective_kind IS NULL OR objective_kind IN ('conversation_task_list'));
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_bear_jobs_one_active_conversation_objective
    ON bear_jobs (bear_id, source_conversation_id, objective_kind)
    WHERE source_conversation_id IS NOT NULL
      AND objective_kind = 'conversation_task_list'
      AND status NOT IN ('completed', 'cancelled');

CREATE INDEX IF NOT EXISTS idx_bear_jobs_source_conversation
    ON bear_jobs (bear_id, source_conversation_id, updated_at DESC)
    WHERE source_conversation_id IS NOT NULL;
