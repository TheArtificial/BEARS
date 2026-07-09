#!/usr/bin/env bash
# Build the sandbox base image (headless bear-armature) used by the
# RUN_SANDBOX provider. See packaging/sandbox-image/Dockerfile.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="${SANDBOX_IMAGE:-bears/sandbox:latest}"
BUILD_SHA="$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo container)"

exec docker build \
    -f "$REPO_ROOT/packaging/sandbox-image/Dockerfile" \
    --build-arg BEAR_ARMATURE_BUILD_SHA="$BUILD_SHA" \
    -t "$TAG" \
    "$REPO_ROOT"
