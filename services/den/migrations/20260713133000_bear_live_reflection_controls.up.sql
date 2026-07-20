ALTER TABLE bears
    ADD COLUMN IF NOT EXISTS live_reflection_stale_after_minutes INTEGER NOT NULL DEFAULT 30,
    ADD COLUMN IF NOT EXISTS live_reflection_activity_threshold INTEGER NOT NULL DEFAULT 20,
    ADD COLUMN IF NOT EXISTS live_reflection_sweep_limit INTEGER NOT NULL DEFAULT 25;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'bears_live_reflection_stale_after_minutes_check'
    ) THEN
        ALTER TABLE bears
            ADD CONSTRAINT bears_live_reflection_stale_after_minutes_check
            CHECK (live_reflection_stale_after_minutes BETWEEN 1 AND 1440);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'bears_live_reflection_activity_threshold_check'
    ) THEN
        ALTER TABLE bears
            ADD CONSTRAINT bears_live_reflection_activity_threshold_check
            CHECK (live_reflection_activity_threshold BETWEEN 1 AND 1000);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'bears_live_reflection_sweep_limit_check'
    ) THEN
        ALTER TABLE bears
            ADD CONSTRAINT bears_live_reflection_sweep_limit_check
            CHECK (live_reflection_sweep_limit BETWEEN 1 AND 100);
    END IF;
END $$;
