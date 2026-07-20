ALTER TABLE bear_job_events
    DROP CONSTRAINT IF EXISTS bear_job_events_event_type_check;

ALTER TABLE bear_job_events
    ADD CONSTRAINT bear_job_events_event_type_check CHECK (event_type IN (
        'job_created',
        'task_added',
        'task_updated',
        'criterion_evaluated',
        'job_blocked',
        'job_completed',
        'job_cancelled',
        'handoff_requested',
        'note_added',
        'run_started',
        'run_finished',
        'focus_selected'
    ));
