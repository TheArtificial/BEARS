CREATE TABLE IF NOT EXISTS bear_profile_model_settings (
    bear_id UUID NOT NULL REFERENCES bears(id) ON DELETE CASCADE,
    profile TEXT NOT NULL CHECK (profile IN ('chat', 'pair', 'curate', 'work', 'watch')),
    model TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (bear_id, profile)
);

CREATE INDEX IF NOT EXISTS idx_bear_profile_model_settings_bear
    ON bear_profile_model_settings (bear_id);
