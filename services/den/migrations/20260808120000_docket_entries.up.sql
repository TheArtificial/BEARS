-- Durable Docket task journals and job notebooks (ADR-0034).

CREATE TABLE bear_docket_entries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    task_id UUID NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    run_id UUID NULL REFERENCES bear_job_runs (id) ON DELETE SET NULL,
    scope TEXT NOT NULL CHECK (scope IN ('task_journal', 'job_notebook')),
    kind TEXT NOT NULL CHECK (kind IN ('outcome', 'finding', 'decision', 'obstacle', 'follow_up', 'milestone', 'question')),
    summary TEXT NOT NULL CHECK (btrim(summary) <> ''),
    body TEXT NULL,
    disposition TEXT NULL CHECK (disposition IS NULL OR disposition IN ('completed', 'no_change', 'delegated', 'blocked', 'failed', 'cancelled')),
    evidence_refs JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(evidence_refs) = 'array'),
    related_task_ids JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(related_task_ids) = 'array'),
    tags JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(tags) = 'array'),
    by_role TEXT NOT NULL CHECK (by_role IN ('chat', 'pair', 'curate', 'work', 'watch', 'system', 'ui')),
    by_agent_id TEXT NULL,
    by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (job_id IS NOT NULL OR task_id IS NOT NULL),
    CHECK (scope <> 'task_journal' OR task_id IS NOT NULL),
    CHECK ((kind = 'outcome') = (disposition IS NOT NULL)),
    CHECK (kind <> 'question' OR by_role = 'pair')
);

CREATE INDEX bear_docket_entries_task_created
    ON bear_docket_entries (task_id, created_at DESC)
    WHERE task_id IS NOT NULL;

CREATE INDEX bear_docket_entries_job_created
    ON bear_docket_entries (job_id, created_at DESC)
    WHERE job_id IS NOT NULL;
