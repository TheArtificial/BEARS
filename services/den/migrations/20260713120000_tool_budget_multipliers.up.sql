-- Native runtime tool-use budget scaling.

ALTER TABLE bears
    ADD COLUMN IF NOT EXISTS default_tool_budget_multiplier DOUBLE PRECISION NULL CHECK (
        default_tool_budget_multiplier IS NULL OR (
            default_tool_budget_multiplier > 0 AND default_tool_budget_multiplier <= 10
        )
    );

COMMENT ON COLUMN bears.default_tool_budget_multiplier IS 'Optional Bear-level multiplier applied to native runtime tool-use budgets after model defaults.';
