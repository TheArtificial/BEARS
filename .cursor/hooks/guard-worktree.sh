#!/bin/bash
# Cursor sessionStart: auto-repair mass worktree deletions before agents run.
set -euo pipefail
exec /workspace/scripts/guard-worktree.sh /workspace
