ALTER TABLE artifacts
    DROP CONSTRAINT artifacts_storage_kind_check;

ALTER TABLE artifacts
    ADD CONSTRAINT artifacts_storage_kind_check
    CHECK (storage_kind IN ('db_text', 'garage_artifacts', 'external_git_commit'));
