#!/bin/bash
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=load-env.sh
. "${ROOT}/scripts/load-env.sh"
SERVICE=$1
if [ -z "$SERVICE" ]; then
  echo "Usage: ./scripts/restart.sh <service>"
  exit 1
fi
docker compose restart "$SERVICE"
