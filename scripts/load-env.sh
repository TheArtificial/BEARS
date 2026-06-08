#!/bin/bash
# Source repo-root `.env` when present (gitignored). Safe to source multiple times.
# Preserves devcontainer host passthrough when `.env` still uses SETME placeholders.
BEARS_ROOT="${BEARS_ROOT:-/workspace}"
ENV_FILE="${BEARS_ENV_FILE:-${BEARS_ROOT}/.env}"

_preserved_openai="${OPENAI_API_KEY:-}"
_preserved_agent_runtime="${AGENT_RUNTIME:-}"

if [ -f "${ENV_FILE}" ]; then
  set -a
  # shellcheck disable=SC1090
  . "${ENV_FILE}"
  set +a
fi

case "${OPENAI_API_KEY:-}" in
  "" | SETME)
    if [ -n "${_preserved_openai}" ]; then
      export OPENAI_API_KEY="${_preserved_openai}"
    fi
    ;;
esac

case "${AGENT_RUNTIME:-}" in
  "")
    if [ -n "${_preserved_agent_runtime}" ]; then
      export AGENT_RUNTIME="${_preserved_agent_runtime}"
    fi
    ;;
esac
