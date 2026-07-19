#!/bin/sh
set -e

mkdir -p /app/data

# `services/bifrost/config.json` is BEARS' source of truth and is schema-valid
# upstream Bifrost config (no BEARS extension keys). Place it where the upstream
# Bifrost process expects to read it. Model availability comes from the live
# provider catalog filtered by the provider-key allowlist in config.json; there
# is no BEARS metadata sidecar.
cp /app/default-config.json /app/data/config.json

exec /app/docker-entrypoint.sh "$@"
