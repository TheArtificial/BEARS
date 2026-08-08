# Docket UI standards

## Execution boundaries

A Docket job is an explicit durable work contract: it owns durable task trees,
journals, retries, delivery policy, and isolated Work runs. Creating or
dispatching one is not the default way for Pair to perform ordinary work.

A Pair session instead has an optional current task that supplies its working
objective. A bounded local delegate is a separate session-level operation and
is not a Docket dispatch. Until local delegation has a workspace-reservation
protocol, it must be read-only. Docket dispatches always use isolated sandbox
checkouts and must not be presented as changes to Pair's attached workspace.

## Entity references

Work surfaces, jobs, tasks, and work runs use one linked reference form:

```text
Kind short-id: title
```

Examples:

```text
Work surface 1a2b3c4d: den-test-github
Job 6f3a9c1d: ▶️ Improve failure reporting
Task a28e91c4: ⚠️ Preserve pending tasks
Run 53b0a7d9: ✅ Improve failure reporting
```

- Every rendered reference is a hyperlink to that entity.
- The visible ID is the first eight lowercase hexadecimal characters of its
  canonical UUID. Render it with a fixed-width (monospace) font.
- Entity URLs use the first 16 lowercase hexadecimal characters. Routes resolve
  that prefix within the entity type and must reject no-match or ambiguous
  prefixes; persistence and service APIs continue to use full UUIDs.
- On operational/dense views, the ID has hover metadata explaining that it is a
  prefix of the entity's full UUID and exposing that UUID.
- Titles are the job/task title, the work-surface display name, and, for runs,
  the owning job title. A missing title uses a concise type-specific fallback.

## Inline status

Jobs, tasks, and runs prefix their title with one status marker:

| State | Marker |
| --- | --- |
| running/active | `▶️` |
| blocked, failed, or timed out | `⚠️` |
| completed/succeeded/done | `✅` |
| pending, queued, or cancelled | none |

The marker is presentation only: state text remains available in views where
operational detail matters, and icons have accessible labels.

Work surfaces do not use execution-status markers until they have a defined
execution lifecycle.

## Retrying a blocked run

A Docket run blocked by a terminal work failure is immutable history, not a
permanent lock on its job. An explicit job-level retry creates a new current
Docket run and then queues a new work run:

```text
Job
├── Docket run 1: blocked (original failure and evidence retained)
└── Docket run 2: running
    └── Work run 2: queued
```

Automatic dispatch continues to exclude blocked jobs. A retry carries completed
and cancelled task state forward, resets interrupted (`in_progress`) tasks to
`pending`, and preserves task- or criterion-local blockers for explicit
resolution. If unpublished changes from the failed work run cannot be recovered,
the UI must warn the operator and require confirmation before starting clean.

Use **Docket run** for the job lifecycle attempt and **work run** for its provider
execution attempt. Retrying a terminal work run within an active Docket run does
not replace the job's current Docket run; explicitly retrying a job after its
Docket run is blocked does.

## Tool lookup references

Docket lookup tools accept a full UUID or an unambiguous UUID prefix. Prefixes
are lowercase hexadecimal after hyphens are ignored and must contain at least
eight characters. Lookup is scoped to the entity type and current Bear before
matching, so a reference cannot cross entity types or reveal another Bear's
entities. `find_task` may additionally narrow the lookup to a job; `find_work_run`
accepts either a run reference or a job reference to list that job's runs.
