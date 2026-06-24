CREATE TABLE IF NOT EXISTS model_selection_options (
    handle TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    selectable BOOLEAN NOT NULL DEFAULT TRUE,
    recommended BOOLEAN NOT NULL DEFAULT FALSE,
    sort_order INTEGER NULL,
    notes TEXT NULL,
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO model_selection_options (
    handle,
    display_name,
    selectable,
    recommended,
    sort_order,
    metadata_json
)
VALUES
    ('openai/gpt-5.5', 'OpenAI GPT-5.5', TRUE, TRUE, 10, '{"context_window":400000,"max_output_tokens":128000,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-5.1', 'OpenAI GPT-5.1', TRUE, TRUE, 20, '{"context_window":400000,"max_output_tokens":128000,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-5', 'OpenAI GPT-5', TRUE, TRUE, 30, '{"context_window":400000,"max_output_tokens":128000,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-5-mini', 'OpenAI GPT-5 mini', TRUE, TRUE, 40, '{"context_window":400000,"max_output_tokens":128000,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-5-nano', 'OpenAI GPT-5 nano', TRUE, FALSE, 50, '{"context_window":400000,"max_output_tokens":128000,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-4.1', 'OpenAI GPT-4.1', TRUE, TRUE, 60, '{"context_window":1047576,"max_output_tokens":32768,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-4.1-mini', 'OpenAI GPT-4.1 mini', TRUE, TRUE, 70, '{"context_window":1047576,"max_output_tokens":32768,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-4.1-nano', 'OpenAI GPT-4.1 nano', TRUE, FALSE, 80, '{"context_window":1047576,"max_output_tokens":32768,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-4o', 'OpenAI GPT-4o', TRUE, TRUE, 90, '{"context_window":128000,"max_output_tokens":16384,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/gpt-4o-mini', 'OpenAI GPT-4o mini', TRUE, TRUE, 100, '{"context_window":128000,"max_output_tokens":16384,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/o4-mini', 'OpenAI o4-mini', TRUE, TRUE, 110, '{"context_window":200000,"max_output_tokens":100000,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/o3', 'OpenAI o3', TRUE, TRUE, 120, '{"context_window":200000,"max_output_tokens":100000,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/o3-mini', 'OpenAI o3-mini', TRUE, FALSE, 130, '{"context_window":200000,"max_output_tokens":100000,"supports_tools":true,"supports_responses_api":true,"supports_vision":false}'::jsonb),
    ('openai/o1', 'OpenAI o1', TRUE, FALSE, 140, '{"context_window":200000,"max_output_tokens":100000,"supports_tools":true,"supports_responses_api":true,"supports_vision":true}'::jsonb),
    ('openai/o1-mini', 'OpenAI o1-mini', TRUE, FALSE, 150, '{"context_window":128000,"max_output_tokens":65536,"supports_tools":true,"supports_responses_api":true,"supports_vision":false}'::jsonb)
ON CONFLICT (handle) DO NOTHING;
