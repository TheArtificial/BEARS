DO $$
DECLARE
    constraint_name text;
BEGIN
    FOR constraint_name IN
        SELECT conname
        FROM pg_constraint
        WHERE conrelid = 'bear_tasks'::regclass
          AND contype = 'c'
          AND pg_get_constraintdef(oid) LIKE '%job_id IS NOT NULL OR session_anchor_id IS NOT NULL%'
    LOOP
        EXECUTE format('ALTER TABLE bear_tasks DROP CONSTRAINT %I', constraint_name);
    END LOOP;
END $$;

ALTER TABLE bear_tasks
    ADD CONSTRAINT bear_tasks_exactly_one_owner
    CHECK ((job_id IS NULL) <> (session_anchor_id IS NULL));
