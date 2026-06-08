#!/bin/sh
set -eu

DATA_DIR="${BEAR_SQLITE_DATA_DIR:-/var/lib/den/bear-sqlite}"
mkdir -p "${DATA_DIR}"
chown -R appuser:appuser "${DATA_DIR}"

exec su-exec appuser /bin/server "$@"
