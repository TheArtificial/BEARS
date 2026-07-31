ALTER TABLE model_selection_options
    DROP COLUMN IF EXISTS calibration_updated_at,
    DROP COLUMN IF EXISTS calibration_sample_count,
    DROP COLUMN IF EXISTS calibration_tokens_per_char;
