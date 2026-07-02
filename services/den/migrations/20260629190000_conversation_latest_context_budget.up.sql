ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS latest_context_budget_json JSONB NULL,
    ADD COLUMN IF NOT EXISTS latest_context_budget_updated_at TIMESTAMPTZ NULL;
