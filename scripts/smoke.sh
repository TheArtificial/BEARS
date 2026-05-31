#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "Running smoke tests..."

RUNNER_SERVICE="bears-memfs-manager"
RUNNER_DIR="/tmp/bears-smoke"

compose_with_env() {
    env -i PATH="$PATH" HOME="$HOME" DOCKER_CONFIG="${DOCKER_CONFIG:-$HOME/.docker}" \
        JWT_SECRET="${JWT_SECRET:-dev-placeholder}" \
        LETTA_SERVER_PASS="${LETTA_SERVER_PASS:-dev-placeholder}" \
        LETTA_API_KEY="${LETTA_API_KEY:-${LETTA_SERVER_PASS:-dev-placeholder}}" \
        OPENAI_API_KEY="${OPENAI_API_KEY:-dev-placeholder}" \
        WEB_SERVER_URL="${WEB_SERVER_URL:-http://localhost:3000}" \
        SESSION_COOKIE_SECURE="${SESSION_COOKIE_SECURE:-false}" \
        DATABASE_URL="${DATABASE_URL:-postgres://bears:bears@bears-postgres:5432/den?sslmode=disable}" \
        LETTA_PG_URI="${LETTA_PG_URI:-postgresql://bears:bears@bears-letta-postgres:5432/letta}" \
        LETTA_BASE_URL="${LETTA_BASE_URL:-http://bears-letta:8283}" \
        BIFROST_BASE_URL="${BIFROST_BASE_URL:-http://bears-bifrost:8080}" \
        LETTA_MEMFS_SERVICE_URL="${LETTA_MEMFS_SERVICE_URL:-http://bears-memfs-manager:8285}" \
        BIFROST_IMAGE="${BIFROST_IMAGE:-bears-bifrost-dev:latest}" \
        DEN_IMAGE="${DEN_IMAGE:-bears-den-dev:latest}" \
        DEN_PULL_POLICY="${DEN_PULL_POLICY:-never}" \
        CODEPOOL_IMAGE="${CODEPOOL_IMAGE:-bears-codepool-dev:latest}" \
        CODEPOOL_PULL_POLICY="${CODEPOOL_PULL_POLICY:-never}" \
        docker compose --profile bundled "$@"
}

compose_with_env exec -T "$RUNNER_SERVICE" sh -lc "rm -rf '$RUNNER_DIR' && mkdir -p '$RUNNER_DIR/tests'"
compose_with_env cp tests/smoke "$RUNNER_SERVICE:$RUNNER_DIR/tests"
API_URL=""
if compose_with_env exec -T bears-den sh -lc 'case "${RUN_API:-false}" in true|1|yes|on) exit 0 ;; *) exit 1 ;; esac' >/dev/null 2>&1; then
    API_URL="http://bears-den:3001"
fi

compose_with_env exec -T "$RUNNER_SERVICE" sh -lc "python -m pip install --quiet pytest requests && cd '$RUNNER_DIR' && BEARS_DEN_URL=http://bears-den:3000 BEARS_API_URL='$API_URL' BEARS_CODEPOOL_URL=http://bears-codepool:3030 BEARS_MEMFS_MANAGER_URL=http://bears-memfs-manager:8285 BEARS_LETTA_URL=http://bears-letta:8283 LETTA_SERVER_PASS='${LETTA_SERVER_PASS:-dev-placeholder}' python -m pytest tests/smoke/ -v"
