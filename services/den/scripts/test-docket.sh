#!/usr/bin/env bash
# Docket's domain test contract. Keep lanes explicit so future domains can
# provide siblings (test-cabinet.sh, test-armature.sh, test-runtime.sh).
set -euo pipefail

lane="${1:-all}"
case "$lane" in
  policy)
    tests=(
      'docket_policy::'
    )
    ;;
  postgres)
    tests=(
      'methods::tests::docket_execute_starts_pair_loop_for_selected_task'
      'methods::tests::blocked_focused_task_ends_docket_control_and_returns_to_chat'
    )
    ;;
  pair-loop)
    tests=(
      'methods::tests::docket_execute_starts_pair_loop_for_selected_task'
    )
    ;;
  recovery)
    tests=(
      'docket_recovery::'
    )
    ;;
  all)
    "$0" postgres
    "$0" pair-loop
    exit 0
    ;;
  *)
    echo "usage: $0 {policy|postgres|pair-loop|recovery|all}" >&2
    exit 2
    ;;
esac

if [[ "$lane" != policy && -z "${DATABASE_URL:-}" ]]; then
  echo "DATABASE_URL is required for Docket $lane tests" >&2
  exit 2
fi

for test_name in "${tests[@]}"; do
  listed="$(cargo test -p den-bearwire "$test_name" -- --list)"
  if ! grep -q "$test_name" <<<"$listed"; then
    echo "Docket $lane test selector discovered no tests: $test_name" >&2
    exit 1
  fi
  echo "Running Docket $lane: $test_name"
  cargo test -p den-bearwire "$test_name" -- --exact
 done
