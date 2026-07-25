#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
baseline="scripts/sqlx-runtime-query-baseline.tsv"
mode="${1:-check}"

scan() {
    git ls-files --cached --others --exclude-standard -- '*.rs' |
        while IFS= read -r file; do
            count=$(awk '
                BEGIN { count = 0; exempt_next = 0 }
                {
                    is_query = $0 ~ /sqlx::query(_as|_scalar)?[[:space:]]*(::<[^>]+>)?[[:space:]]*\(/
                    is_exempt = $0 ~ /sqlx-dynamic:/
                    if (is_query && !is_exempt && !exempt_next) count++
                    exempt_next = is_exempt
                }
                END { if (count > 0) print count }
            ' "$file")
            if [ -n "$count" ]; then
                printf '%s\t%s\n' "$file" "$count"
            fi
        done |
        sort
}

case "$mode" in
    --update-baseline)
        scan > "$baseline"
        echo "Updated $baseline"
        ;;
    check)
        if [ ! -f "$baseline" ]; then
            echo "Missing $baseline; run $0 --update-baseline" >&2
            exit 1
        fi

        current=$(mktemp)
        trap 'rm -f "$current"' EXIT HUP INT TERM
        scan > "$current"

        failures=$(awk -F '\t' '
            NR == FNR { allowed[$1] = $2; next }
            $2 > allowed[$1] {
                printf "%s: %d runtime SQLx queries (baseline %d)\n", $1, $2, allowed[$1]
                failed = 1
            }
            END { exit failed }
        ' "$baseline" "$current" || true)

        if [ -n "$failures" ]; then
            echo "Unchecked static SQLx query count increased:" >&2
            echo "$failures" >&2
            cat >&2 <<'EOF'
Use query!, query_as!, or query_scalar! for static SQL. For genuinely dynamic
SQL, use QueryBuilder where practical or add an immediately preceding comment:

    // sqlx-dynamic: predicates are assembled from typed optional filters.

Do not update the baseline to admit new unchecked static queries.
EOF
            exit 1
        fi
        echo "SQLx runtime-query ratchet passed"
        ;;
    *)
        echo "usage: $0 [check|--update-baseline]" >&2
        exit 2
        ;;
esac

# ponytail: this ratchets per-file counts rather than parsing Rust syntax; it can
# miss a replacement hidden by removing another query in the same file. Upgrade
# to a Rust syntax-aware lint if that loophole becomes material.
