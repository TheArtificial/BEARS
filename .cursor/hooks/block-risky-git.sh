#!/bin/bash
# Block git patterns that leave the worktree in a partially-deleted state.
set -euo pipefail

input="$(cat)"
command="$(printf '%s' "${input}" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("command",""))' 2>/dev/null || true)"

deny() {
  local msg="$1"
  python3 -c 'import json,sys; print(json.dumps({"permission":"deny","user_message":sys.argv[1],"agent_message":sys.argv[1]}))' "${msg}"
  exit 2
}

allow() {
  echo '{"permission":"allow"}'
  exit 0
}

if [ -z "${command}" ]; then
  allow
fi

# Never check out a destructive commit into the working tree.
if printf '%s' "${command}" | grep -Eq 'git checkout[[:space:]]+0819534([[:space:]]|$)'; then
  deny "Blocked: checkout of known destructive commit 0819534."
fi

deleted_count=0
if git -C /workspace rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  deleted_count="$(
    git -C /workspace status --short 2>/dev/null | awk '/^ D / { c++ } END { print c + 0 }'
  )"
fi

# Partial path checkout while many files are already missing — root cause of recurring incidents.
if [ "${deleted_count}" -gt 5 ] && printf '%s' "${command}" | grep -Eq 'git checkout[[:space:]]+([^[:space:]]+)[[:space:]]+--[[:space:]]+'; then
  if ! printf '%s' "${command}" | grep -Eq 'git checkout[[:space:]]+--[[:space:]]+\.'; then
    deny "Blocked partial git checkout while ${deleted_count} tracked files are missing. Run: git checkout -- . (full restore), then make focused edits."
  fi
fi

# Staging everything while the worktree is badly out of sync.
if [ "${deleted_count}" -gt 15 ] && printf '%s' "${command}" | grep -Eq 'git add[[:space:]]+(-A|--all|\.)'; then
  deny "Blocked git add -A while ${deleted_count} tracked files are missing on disk. Run: git checkout -- . first."
fi

allow
