# Docket UI standards

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
