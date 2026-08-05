CREATE UNIQUE INDEX artifact_links_one_primary_output_per_docket_task
    ON artifact_links (target_kind, target_id)
    WHERE target_kind = 'docket_task' AND role = 'primary_output';
