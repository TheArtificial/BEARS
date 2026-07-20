ALTER TABLE bears
    ADD COLUMN IF NOT EXISTS live_reflection_enabled BOOLEAN NOT NULL DEFAULT true;
