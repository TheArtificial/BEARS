-- Promote task-journal entries into a job notebook by reference, without
-- duplicating their semantic content (ADR-0034).

ALTER TABLE bear_docket_entries
    ADD COLUMN source_entry_id UUID NULL
        REFERENCES bear_docket_entries (id) ON DELETE CASCADE;

CREATE UNIQUE INDEX bear_docket_entries_one_notebook_promotion
    ON bear_docket_entries (source_entry_id)
    WHERE source_entry_id IS NOT NULL;

ALTER TABLE bear_docket_entries
    ADD CONSTRAINT bear_docket_entries_promotion_shape CHECK (
        source_entry_id IS NULL
        OR (
            scope = 'job_notebook'
            AND kind <> 'outcome'
            AND task_id IS NOT NULL
        )
    );
