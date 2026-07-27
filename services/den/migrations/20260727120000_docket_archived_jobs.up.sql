ALTER TABLE bear_jobs DROP CONSTRAINT IF EXISTS bear_jobs_status_check;
ALTER TABLE bear_jobs ADD CONSTRAINT bear_jobs_status_check
    CHECK (status IN ('draft', 'ready', 'running', 'blocked', 'completed', 'cancelled', 'archived'));

ALTER TABLE bear_job_events DROP CONSTRAINT IF EXISTS bear_job_events_event_type_check;
ALTER TABLE bear_job_events ADD CONSTRAINT bear_job_events_event_type_check
    CHECK (event_type IN (
        'job_created', 'task_added', 'task_updated', 'criterion_evaluated',
        'job_blocked', 'job_completed', 'job_cancelled', 'job_archived',
        'handoff_requested', 'note_added', 'run_started', 'run_finished',
        'focus_selected'
    ));
