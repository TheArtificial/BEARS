#!/usr/bin/env bash
# Run the Den clippy gate locally, matching CI (.github/workflows/den-clippy.yml).
#
# The v0 gate (docs/roadmap/DEN_CRATE_SPLIT_PLAN.md) denies the default lint set
# (rustc + clippy default groups). pedantic/nursery stay advisory (`warn` via
# `[workspace.lints]`) and are silenced here so they do not gate until v2; run a
# plain `cargo clippy` to see those advisory suggestions.
set -euo pipefail

cd "$(dirname "$0")/../services/den"

export SQLX_OFFLINE="${SQLX_OFFLINE:-true}"

exec cargo clippy --workspace --all-targets -- \
    -D warnings -A clippy::pedantic -A clippy::nursery
