#!/bin/sh
set -eu

manifest="services/den/Cargo.toml"
usage() {
    cat <<'EOF'
Usage: ./scripts/bump-den-version.sh <patch|minor|major|current|X.Y.Z>

Bumps the shared version for the Den Cargo workspace in `services/den/Cargo.toml`.
All Den crates inherit that workspace version via `version.workspace = true`.
EOF
}

if [ "$#" -ne 1 ]; then
    usage >&2
    exit 1
fi

spec="$1"

python3 - "$manifest" "$spec" <<'PY'
import re
import sys
import tomllib
from pathlib import Path

manifest = Path(sys.argv[1])
spec = sys.argv[2].strip()
text = manifest.read_text()
data = tomllib.loads(text)
current = data["workspace"]["package"]["version"]
major, minor, patch = map(int, current.split("."))

if spec == "current":
    print(current)
    raise SystemExit(0)
elif spec == "patch":
    target = f"{major}.{minor}.{patch + 1}"
elif spec == "minor":
    target = f"{major}.{minor + 1}.0"
elif spec == "major":
    target = f"{major + 1}.0.0"
elif re.fullmatch(r"\d+\.\d+\.\d+", spec):
    target = spec
else:
    raise SystemExit(f"Unsupported version spec: {spec}")

if target == current:
    print(current)
    raise SystemExit(0)

lines = text.splitlines(keepends=True)
in_workspace_package = False
replaced = False
for idx, line in enumerate(lines):
    stripped = line.strip()
    if stripped == "[workspace.package]":
        in_workspace_package = True
        continue
    if in_workspace_package and stripped.startswith("["):
        in_workspace_package = False
    if in_workspace_package and stripped.startswith("version"):
        lines[idx] = re.sub(r'"\d+\.\d+\.\d+"', f'"{target}"', line, count=1)
        replaced = True
        break

if not replaced:
    raise SystemExit(f"Could not update [workspace.package] version in {manifest}")

manifest.write_text("".join(lines))
print(f"{current} -> {target}")
PY
