#!/usr/bin/env bash
# Docket's domain test contract. Keep lanes explicit so future domains can
# provide siblings (test-cabinet.sh, test-armature.sh, test-runtime.sh).
set -euo pipefail

lane="${1:-all}"

run_test() {
  local package="$1"
  local selector="$2"
  local features="${3:-}"
  local args=(test -p "$package")
  if [[ -n "$features" ]]; then
    args+=(--features "$features")
  fi
  args+=("$selector" -- --list)
  local listed
  listed="$(cargo "${args[@]}")"
  if ! grep -q "$selector" <<<"$listed"; then
    echo "Docket $lane test selector discovered no tests: $package $selector" >&2
    exit 1
  fi
  echo "Running Docket $lane: cargo test -p $package${features:+ --features $features} $selector"
  args=(test -p "$package")
  if [[ -n "$features" ]]; then
    args+=(--features "$features")
  fi
  args+=("$selector")
  cargo "${args[@]}"
}

case "$lane" in
  policy)
    # Pure control-gate tests: no database or worker is required.
    run_test den-docket execution_control_tests::
    ;;
  postgres)
    : "${DATABASE_URL:?DATABASE_URL is required for Docket postgres tests}"
    run_test den-docket execution_attempt_tests::
    run_test den-bearwire methods::tests::docket_execute_starts_pair_loop_for_selected_task
    run_test den-bearwire methods::tests::blocked_focused_task_ends_docket_control_and_returns_to_chat
    ;;
  pair-loop)
    : "${DATABASE_URL:?DATABASE_URL is required for Docket pair-loop tests}"
    run_test den-bearwire methods::tests::docket_execute_starts_pair_loop_for_selected_task
    run_test den-bearwire methods::tests::focused_pair_loop_continues_across_two_bounded_slices test-fixtures
    ;;
  recovery)
    : "${DATABASE_URL:?DATABASE_URL is required for Docket recovery tests}"
    run_test den-docket execution_attempt_tests::released_running_attempt_is_fenced_idempotent_and_not_startable
    ;;
  all)
    "$0" policy
    "$0" postgres
    "$0" pair-loop
    "$0" recovery
    ;;
  *)
    echo "usage: $0 {policy|postgres|pair-loop|recovery|all}" >&2
    exit 2
    ;;
esac
