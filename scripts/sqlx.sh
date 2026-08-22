#!/bin/bash
set -euo pipefail

ROOT="/workspace"
DATABASE_URL="postgres://bears:bears@bears-postgres:5432/den?sslmode=disable"

usage() {
  cat <<'EOF'
Usage: ./scripts/sqlx.sh <sqlx-command> [arguments...]

Start bundled Postgres if needed, then run `cargo sqlx` from the Den workspace
with a database URL reachable from this workspace container.

Examples:
  ./scripts/sqlx.sh migrate run
  ./scripts/sqlx.sh prepare --workspace -- --all-targets
  ./scripts/sqlx.sh prepare --check --workspace -- --all-targets
EOF
}

if [ "$#" -eq 0 ]; then
  usage >&2
  exit 2
fi

"${ROOT}/scripts/smoke-stack.sh" --infra
cd "${ROOT}/services/den"
exec env DATABASE_URL="${DATABASE_URL}" cargo sqlx "$@"
