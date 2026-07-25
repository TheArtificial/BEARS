ALTER TABLE bear_tasks
    ADD COLUMN IF NOT EXISTS assigned_to_role TEXT NULL
        CHECK (assigned_to_role IS NULL OR assigned_to_role IN ('chat', 'pair', 'curate', 'work', 'watch'));
