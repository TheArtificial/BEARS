#!/bin/bash
# Reject SQLx migrations that reuse a numeric version for another description.
set -euo pipefail

check_paths() {
  local path name prefix description seen
  declare -A descriptions=()
  declare -A conflicts=()

  while IFS= read -r -d '' path; do
    name="${path##*/}"
    [[ "${name}" =~ ^([0-9]+)_(.+)\.(up|down)\.sql$ ]] || continue
    prefix="${BASH_REMATCH[1]}"
    description="${BASH_REMATCH[2]}"
    seen="${descriptions[${prefix}]-}"

    if [[ -n "${seen}" && "${seen}" != "${description}" ]]; then
      conflicts["${prefix}"]="${seen} and ${description}"
    else
      descriptions["${prefix}"]="${description}"
    fi
  done

  if ((${#conflicts[@]})); then
    echo "SQLx migration version conflict(s):" >&2
    for prefix in "${!conflicts[@]}"; do
      echo "  ${prefix}: ${conflicts[${prefix}]}" >&2
    done
    echo "Create migrations with ./scripts/sqlx.sh migrate add <description>; do not choose numeric prefixes manually." >&2
    return 1
  fi
}

case "${1-}" in
  --staged)
    check_paths < <(git ls-files --cached -z -- services/den/migrations)
    ;;
  --self-check)
    check_paths < <(printf '%s\0' \
      '20260101000000_first.up.sql' \
      '20260101000000_first.down.sql' \
      '20260101000001_second.up.sql')
    if check_paths < <(printf '%s\0' \
      '20260101000000_first.up.sql' \
      '20260101000000_second.up.sql'); then
      echo "expected duplicate SQLx migration prefix check to fail" >&2
      exit 1
    fi
    ;;
  '')
    check_paths < <(find services/den/migrations -type f -name '*.sql' -print0)
    ;;
  *)
    echo "usage: $0 [--staged|--self-check]" >&2
    exit 2
    ;;
esac
