-- ADR-0047 §7 (2026-07-30 amendment): per-model chars→tokens calibration lives
-- in the Den model registry alongside resolved model metadata. Bifrost owns
-- usage truth; Den mirrors an aggregate ratio (EMA of observed prompt tokens
-- per assembled prompt character) per model handle for estimation and policy.
ALTER TABLE model_selection_options
    ADD COLUMN IF NOT EXISTS calibration_tokens_per_char DOUBLE PRECISION NULL,
    ADD COLUMN IF NOT EXISTS calibration_sample_count BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS calibration_updated_at TIMESTAMPTZ NULL;
