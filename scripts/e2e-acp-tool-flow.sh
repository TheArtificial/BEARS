#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "Building bear-armature for ACP/BearWire e2e..."
cargo build --manifest-path tools/bear-armature/Cargo.toml

export BEAR_ARMATURE_BIN="${BEAR_ARMATURE_BIN:-$ROOT/tools/bear-armature/target/debug/bear-armature}"
python3 -m pytest tests/e2e/test_acp_bearwire_tool_flow.py -v
