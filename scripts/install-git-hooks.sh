#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK_SRC="${ROOT}/scripts/git-hooks/pre-commit"
HOOK_DST="${ROOT}/.git/hooks/pre-commit"

if [ ! -d "${ROOT}/.git/hooks" ]; then
  echo "install-git-hooks: not a git repository (${ROOT})" >&2
  exit 1
fi

install -m 0755 "${HOOK_SRC}" "${HOOK_DST}"
echo "install-git-hooks: installed ${HOOK_DST}"
