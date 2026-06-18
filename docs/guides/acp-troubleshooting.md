# ACP Troubleshooting Runbook

This runbook covers the Bear Den ACP direct path:

```text
Editor ⇄ bear-armature ⇄ Den ACP gateway ⇄ Den native agent loop ⇄ Bifrost
```

ACP `pair` profile traffic runs in-process when `AGENT_RUNTIME=native` (default). It does not route through an external harness.

---

## 1. Verify deployed versions

Check Den:

```bash
curl -s "$DEN_API_URL/version"
```

Check adapter startup in the editor logs:

```text
bear-armature: starting version=... build_git_sha=... local_head_sha=...
```

If Den and adapter are not both current, fix that first. Many ACP failures are version skew.

## 1a. Inspect bear environment and status

The ACP adapter exposes a single read-only diagnostic tool, `bear_environment`, plus `/status` as a compact human rendering of the same underlying environment snapshot.

Use these when you need to distinguish between:

- adapter runtime problems
- Den reachability problems
- session/MCP registration problems
- host browser bridge configuration problems
- local Chrome fallback problems

Expected behavior:

- `bear_environment` returns structured environment state for the current bear/session/runtime.
- `/status` renders a compact summary from the same shared snapshot.
- If Den cannot be reached, `/status` should still show meaningful degraded status rather than failing silently.

For host browser bridge debugging, the most relevant fields are:

- `browser.active_source`
- `services.den`
- `environment_variants.acp_adapter.host_browser_bridge_env`
- `environment_variants.acp_adapter.session_mcp`
- `diagnostics.status`
- `diagnostics.warnings`
- `diagnostics.errors`

---

## 2. Basic chat diagnostic

Prompt:

```text
Reply with exactly: hello from bear
```

Expected adapter log:

```text
bear-armature: Den stream summary ... event_types={"assistant_text_delta": ..., "turn_complete": 1} ... saw_assistant_output=true
```

Expected Den log:

```text
ACP Letta stream summary ... mapped_events>0 ... adapter_event_types={"assistant_text_delta": ...}
```

(Log line name is historical; with native runtime the upstream is the in-process agent loop, not Letta HTTP.)

If basic chat fails, do not debug file tools yet.

---

## 3. File read diagnostic

Prompt with an absolute path under the current workspace:

```text
Read /absolute/path/to/small-file.txt and summarize it.
```

Expected flow:

1. Native loop emits a tool request mapped to adapter event `tool_request`.
2. Adapter logs `requesting permission` if approval is required.
3. Adapter calls ACP client `fs/read_text_file` if the client advertises it.
4. Adapter logs fallback only if client does not advertise `fs.readTextFile`.
5. Adapter posts result to Den.
6. Den continues the same in-process turn with the tool result.
7. Den streams assistant text deltas to the adapter.

Useful adapter log snippets:

```text
bear-armature: requesting permission session_id=... tool_call_id=... tool_name=... path=...
bear-armature: client fs/read_text_file path=... bytes=... duration_ms=...
bear-armature: posted tool result session_id=... tool_call_id=... response=...
bear-armature: Den stream summary ...
```

Useful Den log snippets:

```text
ACP tool request registered ... tool_call_id=... tool_name=...
ACP tool result received ... body_tool_call_id=... body_approval_request_id=...
ACP Letta stream summary ... native_message_types=... adapter_event_types=...
```

Expected user-visible tool UX:

- The ACP client should show a human-readable tool card, such as `Reading /absolute/path/to/small-file.txt`, not a generic `tool_call` title.
- Permission prompts should include the concrete target and risk, such as the path, URL host, command/cwd, memory scope, or plan id.
- Raw `args` may be attached as diagnostic/raw input, but visible content should prefer Den `display.title`, `display.subtitle`, `display.approval_summary`, and bounded summaries.
- If a new tool renders generically, verify that its Den/ACP descriptor includes display metadata and that the adapter is consuming `event.display`.

---

## 4. Common failures

### Invalid provider tool name

Symptom:

```text
Invalid 'tools[0].name': string does not match pattern
```

Cause: Den sent a provider tool name with `.`, `/`, or whitespace.

Expected provider name:

```text
fs_read_text_file
```

Not:

```text
fs.read_text_file
fs/read_text_file
```

See `docs/architecture/adr/provider-safe-tool-naming.md`.

### Empty turn with tool requests

Symptom:

```text
completed the turn without producing displayable ACP output
mapped_events=0
```

Cause: Den did not map native runtime tool events to adapter `tool_request` events.

Actions:

1. Set Den env var:

```bash
ACP_DEBUG_EVENT_SAMPLE_CHARS=8000
```

2. Restart Den.
3. Reproduce once.
4. Copy one full unmapped event sample from Den logs, keeping `tool_call_id`, `tool_name`, and argument fields intact.

### Tool return while turn still active

Symptom:

```text
Cannot send a new message: Another request is currently being processed
```

Cause: Adapter or Den posted a continuation before the active turn accepted it, or a duplicate prompt raced the in-flight turn.

Expected behavior: tool results settle against the registered `tool_call_id` for the active turn; new prompts should queue or reject per active-turn policy.

### Invalid tool call IDs

Symptom:

```text
Invalid tool call IDs. Expected '[call_...]', but received '[fs_read_text_file]'
```

Cause: Den or the adapter sent the provider tool name instead of the runtime `tool_call_id` in the tool return.

Expected: tool result payloads reference the original `tool_call_id` from the `tool_request` event.

### Approval JSON shape rejected

Symptom:

```text
Unable to extract tag using discriminator 'type'
```

Cause: Den sent an approval return without the expected structured approval payload.

Verify the adapter posts tool results in the shape Den's ACP gateway expects (see gateway tests under `services/den/src/api/acp/`).

### Missing file path

Symptom:

```text
requested fs_read_text_file without a path argument
```

Cause: Model emitted a tool call without a string `path`, or Den parsed the wrong field.

If debug samples show argument fragments, Den should accumulate until valid JSON appears.

---

## 5. Safe raw sample collection

Set:

```bash
ACP_DEBUG_EVENT_SAMPLE_CHARS=8000
```

Then find in Den logs:

```text
ACP Letta stream summary
unmapped_event_samples=[...]
```

Redact secrets and local usernames if desired, but preserve:

- event/message type
- `id`
- `tool_call_id`
- `tool_name`
- `arguments` / `input` / `args`

---

## 6. Protocol boundaries

Do not confuse these layers:

```text
Editor ⇄ adapter: ACP JSON-RPC over stdio
Adapter ⇄ Den: Den-private HTTPS/SSE transport
Den: in-process native agent loop + Bifrost /v1 streaming
```

Den ⇄ adapter event names include:

```text
assistant_text_delta
status_text
tool_request
conversation_resolved
turn_complete
error
```

These are not raw ACP messages; the adapter translates them into ACP `session/update` and client requests.
