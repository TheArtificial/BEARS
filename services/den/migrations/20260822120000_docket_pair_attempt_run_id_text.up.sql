ALTER TABLE docket_execution_attempts
    ALTER COLUMN pair_run_id TYPE TEXT USING pair_run_id::TEXT;
