#!/bin/bash
set -u -o pipefail

ROOT="/workspace"
LOG_DIR="${ROOT}/.devcontainer/logs"

if [ -x "${ROOT}/scripts/guard-worktree.sh" ]; then
  "${ROOT}/scripts/guard-worktree.sh" "${ROOT}" || true
fi
if [ -x "${ROOT}/scripts/install-git-hooks.sh" ]; then
  "${ROOT}/scripts/install-git-hooks.sh" || true
fi
LOG_FILE="${LOG_DIR}/startup.log"
STATUS_FILE="${LOG_DIR}/startup.status"

mkdir -p "${LOG_DIR}"
: >"${LOG_FILE}"

log() {
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "${LOG_FILE}"
}

set_status() {
  printf '%s\n' "$1" >"${STATUS_FILE}"
}

run_logged() {
  log "+ $*"
  "$@" >>"${LOG_FILE}" 2>&1
}

# shellcheck source=/workspace/scripts/load-env.sh
. "${ROOT}/scripts/load-env.sh"

export JWT_SECRET="${JWT_SECRET:-dev-placeholder}"
export OPENAI_API_KEY="${OPENAI_API_KEY:-dev-placeholder}"
export WEB_SERVER_URL="${WEB_SERVER_URL:-http://localhost:3000}"
export SESSION_COOKIE_SECURE="${SESSION_COOKIE_SECURE:-false}"
export DATABASE_URL="${DATABASE_URL:-postgres://bears:bears@bears-postgres:5432/den?sslmode=disable}"
export LLM_API_URL="${LLM_API_URL:-http://bears-bifrost:8080/v1}"
export BIFROST_IMAGE="${BIFROST_IMAGE:-bears-bifrost-dev:latest}"
export DEN_IMAGE="${DEN_IMAGE:-bears-den-dev:latest}"
export AGENT_RUNTIME="${AGENT_RUNTIME:-native}"

set_status "starting"
log "Starting BEARS devcontainer stack (native runtime)"

build_ok=1
if ! run_logged docker build -t "${BIFROST_IMAGE}" "${ROOT}/services/bifrost"; then
  build_ok=0
  log "Bifrost image build failed"
fi
if ! run_logged docker build --build-arg SQLX_OFFLINE=true -t "${DEN_IMAGE}" "${ROOT}/services/den"; then
  build_ok=0
  log "Den image build failed"
fi

log "Starting and verifying bundled Postgres via smoke-stack"
if ! run_logged "${ROOT}/scripts/smoke-stack.sh" --infra; then
  set_status "postgres_start_failed"
  log "Bundled Postgres startup failed; devcontainer remains usable. See ${LOG_FILE}."
  exit 0
fi

log "Running dev/smoke seed profile"
seed_ok=0
if ! run_logged "${ROOT}/scripts/seed-dev.sh" smoke; then
  seed_ok=0
  log "Seed failed; devcontainer remains usable. Rerun with: /workspace/scripts/seed-dev.sh smoke"
  log "Detailed seed output is in ${LOG_FILE}."
else
  seed_ok=1
fi

log "Starting remaining BEARS services"
stack_ok=0
if [ "${build_ok}" != "1" ]; then
  log "Skipping full stack startup because one or more local source image builds failed"
elif ! run_logged docker compose -f "${ROOT}/docker-compose.yaml" -f "${ROOT}/docker-compose.dev.yaml" up -d --force-recreate bears-bifrost bears-den; then
  stack_ok=0
  log "Full stack startup failed; devcontainer remains usable. See ${LOG_FILE}."
else
  stack_ok=1
fi

if [ "${build_ok}" != "1" ]; then
  set_status "local_image_build_failed"
  log "Local Den/Bifrost image build failed; full stack was not started"
elif [ "${seed_ok}" = "1" ] && [ "${stack_ok}" = "1" ]; then
  set_status "ok"
  log "Devcontainer stack started and seed profile applied successfully"
elif [ "${seed_ok}" = "1" ]; then
  set_status "stack_failed_after_seed"
  log "Seed profile applied, but full stack startup failed"
elif [ "${stack_ok}" = "1" ]; then
  set_status "seed_failed"
  log "Full stack started, but seed profile failed"
else
  set_status "seed_and_stack_failed"
  log "Both seed profile and full stack startup failed"
fi
exit 0
