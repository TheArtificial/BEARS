-- ADR-0050: operator-configurable agent loop control levels.
-- Model registry supplies defaults; these columns allow Bear-level and stance/profile-level overrides.

ALTER TABLE bears
    ADD COLUMN IF NOT EXISTS default_agent_loop_control_level TEXT NULL CHECK (
        default_agent_loop_control_level IS NULL OR default_agent_loop_control_level IN (
            'light', 'standard', 'careful', 'strict'
        )
    );

ALTER TABLE bear_profile_model_settings
    ADD COLUMN IF NOT EXISTS agent_loop_control_level TEXT NULL CHECK (
        agent_loop_control_level IS NULL OR agent_loop_control_level IN (
            'light', 'standard', 'careful', 'strict'
        )
    );

COMMENT ON COLUMN bears.default_agent_loop_control_level IS 'Optional Bear-level default agent loop control level overriding model registry defaults.';
COMMENT ON COLUMN bear_profile_model_settings.agent_loop_control_level IS 'Optional stance/profile-level agent loop control override.';
