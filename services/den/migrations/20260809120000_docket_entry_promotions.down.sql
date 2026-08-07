DROP INDEX IF EXISTS bear_docket_entries_one_notebook_promotion;

ALTER TABLE bear_docket_entries
    DROP CONSTRAINT IF EXISTS bear_docket_entries_promotion_shape,
    DROP COLUMN IF EXISTS source_entry_id;
