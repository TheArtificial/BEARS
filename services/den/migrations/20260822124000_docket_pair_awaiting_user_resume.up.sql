-- Durable Pair question/response provenance for fenced awaiting-user attempts.
CREATE TABLE docket_pair_awaiting_user_questions (
    execution_attempt_id UUID NOT NULL REFERENCES docket_execution_attempts (id) ON DELETE CASCADE,
    question_key UUID NOT NULL,
    question_reference TEXT NOT NULL CHECK (btrim(question_reference) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (execution_attempt_id, question_key)
);

CREATE TABLE docket_pair_awaiting_user_responses (
    execution_attempt_id UUID NOT NULL,
    question_key UUID NOT NULL,
    response_key UUID NOT NULL UNIQUE,
    response_reference TEXT NOT NULL CHECK (btrim(response_reference) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (execution_attempt_id, question_key, response_key),
    FOREIGN KEY (execution_attempt_id, question_key)
        REFERENCES docket_pair_awaiting_user_questions (execution_attempt_id, question_key)
        ON DELETE CASCADE
);

COMMENT ON TABLE docket_pair_awaiting_user_questions IS
    'Exact Pair question that placed a canonical execution attempt into awaiting_user.';
COMMENT ON TABLE docket_pair_awaiting_user_responses IS
    'Trusted authenticated response/resume provenance for a Pair awaiting-user question.';
