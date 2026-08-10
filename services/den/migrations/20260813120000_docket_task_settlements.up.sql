-- A task's terminal outcome is canonical, while a job run remains optional
-- provenance for the work that settled it.
ALTER TABLE bear_tasks
    ADD COLUMN IF NOT EXISTS settled_by_entry_id UUID NULL
    REFERENCES bear_docket_entries (id) ON DELETE SET NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_bear_tasks_settled_by_entry
    ON bear_tasks (settled_by_entry_id)
    WHERE settled_by_entry_id IS NOT NULL;

COMMENT ON COLUMN bear_tasks.settled_by_entry_id IS
    'Authoritative task-journal outcome entry that settled this task; nullable while pending.';
