#!/bin/bash
# Detect mass deletion of tracked files (worktree out of sync with git) and repair.
# Called from devcontainer startup and Cursor sessionStart hook.
set -euo pipefail

ROOT="${1:-/workspace}"
THRESHOLD="${WORKTREE_DELETE_THRESHOLD:-15}"
LOG_DIR="${ROOT}/.devcontainer/logs"
LOG_FILE="${LOG_DIR}/worktree-guard.log"

mkdir -p "${LOG_DIR}"

log() {
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "${LOG_FILE}"
}

if ! git -C "${ROOT}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  exit 0
fi

deleted_count="$(
  git -C "${ROOT}" status --short 2>/dev/null | awk '/^ D / { c++ } END { print c + 0 }'
)"

if [ "${deleted_count}" -lt "${THRESHOLD}" ]; then
  exit 0
fi

log "worktree guard: ${deleted_count} tracked files missing on disk (threshold ${THRESHOLD}); restoring from HEAD"
if git -C "${ROOT}" checkout -- . >>"${LOG_FILE}" 2>&1; then
  log "worktree guard: restore complete"
else
  log "worktree guard: restore FAILED — see ${LOG_FILE}"
  exit 1
fi
