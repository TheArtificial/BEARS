#!/usr/bin/env sh
set -eu

RUN_ID="${1:-}"
if [ -z "$RUN_ID" ]; then
  echo "usage: $0 <bearwire-run-id>" >&2
  exit 64
fi

CONTAINER="${BEARS_POSTGRES_CONTAINER:-bears-stack-bears-postgres-1}"
DB_USER="${BEARS_POSTGRES_USER:-bears}"
DB_NAME="${BEARS_POSTGRES_DB:-den}"

psql_cmd() {
  docker exec "$CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 "$@"
}

if ! docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
  echo "Postgres container '$CONTAINER' is not running. Set BEARS_POSTGRES_CONTAINER if needed." >&2
  exit 69
fi

if ! psql_cmd -Atc "select to_regclass('public.bearwire_runs') is not null and to_regclass('public.bearwire_events') is not null" | grep -qx t; then
  echo "BearWire tables are not present in $DB_NAME on $CONTAINER. This Den may be an older schema or a different environment." >&2
  echo "Available BearWire-like tables:" >&2
  psql_cmd -c "select schemaname, tablename from pg_tables where tablename like 'bearwire%' order by tablename" >&2 || true
  exit 69
fi

SESSION_ID="$(psql_cmd -v run_id="$RUN_ID" -Atc "select session_id from bearwire_runs where run_id = :'run_id' limit 1")"
if [ -z "$SESSION_ID" ]; then
  echo "No bearwire_runs row found for run_id '$RUN_ID'." >&2
  exit 66
fi

printf '# BearWire run\n'
psql_cmd -v run_id="$RUN_ID" -x -c "select run_id, session_id, state, terminal_reason, created_at, updated_at from bearwire_runs where run_id = :'run_id'"

printf '\n# Obligations\n'
psql_cmd -v run_id="$RUN_ID" -x -c "select kind, expected_client_method, tool_call_id, permission_id, state, request_payload, result_payload, created_at, updated_at from bearwire_run_obligations where run_id = :'run_id' order by created_at"

printf '\n# Client results\n'
psql_cmd -v run_id="$RUN_ID" -x -c "select obligation_kind, obligation_id, payload_json, created_at from bearwire_client_results where run_id = :'run_id' order by created_at"

printf '\n# Event timeline\n'
psql_cmd -v run_id="$RUN_ID" -x -c "select sequence_no, event_type, event_json->>'run_id' as run_id, event_json->>'subject' as subject, event_json->'data' as data, created_at from bearwire_events where event_json->>'run_id' = :'run_id' order by sequence_no"
