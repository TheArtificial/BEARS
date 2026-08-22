-- Replace the legacy direct task owner with the one Pair-binding relation.
-- Each legacy anchored task receives an unreleased binding before the column goes away.
INSERT INTO bear_pair_task_attachments (task_id, session_id, attached_at)
SELECT id, session_anchor_id, created_at
FROM bear_tasks
WHERE session_anchor_id IS NOT NULL
ON CONFLICT (task_id) DO NOTHING;

ALTER TABLE bear_tasks DROP CONSTRAINT IF EXISTS bear_tasks_exactly_one_owner;
DROP INDEX IF EXISTS idx_bear_tasks_session_anchor;
ALTER TABLE bear_tasks DROP COLUMN session_anchor_id;

-- A task may be job-backed or standalone. Pair actionability is represented
-- exclusively through bear_pair_task_attachments.
