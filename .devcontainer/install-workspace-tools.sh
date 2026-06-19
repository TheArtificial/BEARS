#!/bin/bash
set -euo pipefail

force_compile=0
for arg in "$@"; do
  case "$arg" in
    --force-compile)
      force_compile=1
      ;;
    -h|--help)
      cat <<'USAGE'
Usage: .devcontainer/install-workspace-tools.sh [--force-compile]

Options:
  --force-compile   Build bear-armature from /workspace/tools/bear-armature and install it,
                    skipping update manifests and release downloads.
  -h, --help        Show this help.
USAGE
      exit 0
      ;;
    *)
      echo "install-workspace-tools.sh: unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

export DEBIAN_FRONTEND=noninteractive
export RUST_VERSION="${RUST_VERSION:-1.92.0}"
export RUSTUP_HOME=/usr/local/rustup
export CARGO_HOME=/usr/local/cargo
export PATH="${CARGO_HOME}/bin:${PATH}"

needs_apt=0
for cmd in bash curl git gh docker python3 clang ld.lld node npm; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    needs_apt=1
  fi
done

if ! command -v node >/dev/null 2>&1 || ! node -e 'process.exit(Number(process.versions.node.split(".")[0]) >= 22 ? 0 : 1)' >/dev/null 2>&1; then
  needs_apt=1
fi

if ! python3 - <<'PY' >/dev/null 2>&1
import pytest
import requests
PY
then
  needs_apt=1
fi

if [ "$needs_apt" = "1" ]; then
  apt-get update
  apt-get install -y \
    curl git bash ca-certificates build-essential clang lld pkg-config libssl-dev \
    python3 python3-pytest python3-requests \
    docker.io
  if ! command -v gh >/dev/null 2>&1; then
    mkdir -p -m 755 /etc/apt/keyrings
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg -o /etc/apt/keyrings/githubcli-archive-keyring.gpg
    chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list
    apt-get update
    apt-get install -y gh
  fi
  if ! command -v node >/dev/null 2>&1 || ! node -e 'process.exit(Number(process.versions.node.split(".")[0]) >= 22 ? 0 : 1)' >/dev/null 2>&1; then
    curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
    apt-get install -y nodejs
  fi
  rm -rf /var/lib/apt/lists/*
fi

if ! docker compose version >/dev/null 2>&1; then
  mkdir -p /usr/local/lib/docker/cli-plugins
  arch="$(uname -m)"
  case "$arch" in
    x86_64) compose_arch="x86_64" ;;
    aarch64|arm64) compose_arch="aarch64" ;;
    *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
  esac
  curl -fsSL "https://github.com/docker/compose/releases/download/v2.29.7/docker-compose-linux-${compose_arch}" \
    -o /usr/local/lib/docker/cli-plugins/docker-compose
  chmod +x /usr/local/lib/docker/cli-plugins/docker-compose
fi

if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
    | bash -s -- -y --no-modify-path --profile minimal --default-toolchain "$RUST_VERSION"
  rustup component add clippy rustfmt
fi

toolchain_dir="$(find "${RUSTUP_HOME}/toolchains" -maxdepth 1 -mindepth 1 -type d | head -n 1)"
for name in cargo cargo-clippy clippy-driver rustc rustdoc rustfmt; do
  bin="${toolchain_dir}/bin/${name}"
  [ -x "$bin" ] || continue
  ln -sf "$bin" "${CARGO_HOME}/bin/${name}"
  ln -sf "$bin" "/usr/local/bin/${name}"
done

printf 'export PATH="%s/bin:$PATH"\n' "${CARGO_HOME}" > /etc/profile.d/cargo.sh
chmod -R a+w "${RUSTUP_HOME}" "${CARGO_HOME}"

install_bear_armature_binary() {
  local url="$1"
  local install_dir="$2"
  local expected_sha256="${3:-}"
  local expected_size="${4:-}"
  local tmp actual_sha256 actual_size

  tmp="$(mktemp)"
  echo "bear-armature: downloading ${url}"
  if ! curl -fsSL "${url}" -o "${tmp}"; then
    rm -f "${tmp}"
    return 1
  fi

  if [ -n "${expected_size}" ]; then
    actual_size="$(wc -c < "${tmp}" | tr -d '[:space:]')"
    if [ "${actual_size}" != "${expected_size}" ]; then
      rm -f "${tmp}"
      echo "bear-armature: size mismatch for ${url}: expected ${expected_size}, got ${actual_size}" >&2
      return 1
    fi
  fi

  if [ -n "${expected_sha256}" ]; then
    actual_sha256="$(sha256sum "${tmp}" | awk '{print $1}')"
    if [ "${actual_sha256}" != "${expected_sha256}" ]; then
      rm -f "${tmp}"
      echo "bear-armature: SHA-256 mismatch for ${url}: expected ${expected_sha256}, got ${actual_sha256}" >&2
      return 1
    fi
  fi

  chmod 0755 "${tmp}"
  if ! "${tmp}" --help >/dev/null; then
    rm -f "${tmp}"
    echo "bear-armature: downloaded binary failed to run on this system; leaving any existing install untouched" >&2
    return 1
  fi

  mkdir -p "${install_dir}"
  install -m 0755 "${tmp}" "${install_dir}/bear-armature"
  ln -sf "${install_dir}/bear-armature" "${install_dir}/bears-acp-adapter"
  rm -f "${tmp}"
  echo "bear-armature: installed to ${install_dir}/bear-armature (symlink: ${install_dir}/bears-acp-adapter)"
}

install_bear_armature_from_source() {
  local install_dir="$1"
  if [ -f /workspace/tools/bear-armature/Cargo.toml ]; then
    echo "bear-armature: falling back to local source build" >&2
    if ! cargo build --release --locked --manifest-path /workspace/tools/bear-armature/Cargo.toml; then
      echo "bear-armature: locked source build failed; retrying without --locked so Cargo.lock can be refreshed for this checkout" >&2
      cargo build --release --manifest-path /workspace/tools/bear-armature/Cargo.toml
    fi
    ln -sf /workspace/tools/bear-armature/target/release/bear-armature "${install_dir}/bear-armature"
    ln -sf "${install_dir}/bear-armature" "${install_dir}/bears-acp-adapter"
  else
    echo "bear-armature: set BEAR_ARMATURE_MANIFEST_URL or install manually" >&2
  fi
}

manifest_url_for_channel() {
  local channel="$1"
  local triple="$2"
  if [ -n "${BEAR_ARMATURE_MANIFEST_URL:-}" ]; then
    printf '%s\n' "${BEAR_ARMATURE_MANIFEST_URL}"
    return 0
  fi
  if [ -n "${DEN_ACP_ADAPTER_MANIFEST_URL:-}" ]; then
    printf '%s\n' "${DEN_ACP_ADAPTER_MANIFEST_URL}"
    return 0
  fi
  if [ -n "${BEARS_ACP_ADAPTER_MANIFEST_URL:-}" ]; then
    printf '%s\n' "${BEARS_ACP_ADAPTER_MANIFEST_URL}"
    return 0
  fi
  printf 'https://bears-ai.github.io/bear-den/bear-armature/%s/%s.json\n' "${channel}" "${triple}"
}

legacy_manifest_url_for_channel() {
  local channel="$1"
  local triple="$2"
  printf 'https://bears-ai.github.io/bear-den/bears-acp-adapter/%s/%s.json\n' "${channel}" "${triple}"
}

install_bear_armature() {
  local version="${BEAR_ARMATURE_VERSION:-${DEN_ACP_ADAPTER_VERSION:-${BEARS_ACP_ADAPTER_VERSION:-}}}"
  local channel="${BEAR_ARMATURE_CHANNEL:-${DEN_ACP_ADAPTER_CHANNEL:-${BEARS_ACP_ADAPTER_CHANNEL:-stable}}}"
  local install_dir="${BEAR_ARMATURE_INSTALL_DIR:-${DEN_ACP_ADAPTER_INSTALL_DIR:-${BEARS_ACP_ADAPTER_INSTALL_DIR:-/usr/local/bin}}}"
  local arch triple asset manifest_url manifest_tmp legacy_manifest_url url sha256 size parsed_version cargo_version

  arch="$(uname -m)"
  case "${arch}" in
    x86_64|amd64) triple="x86_64-unknown-linux-gnu" ;;
    aarch64|arm64) triple="aarch64-unknown-linux-gnu" ;;
    *) echo "bear-armature: unsupported Linux architecture: ${arch}" >&2; return 0 ;;
  esac

  asset="bear-armature-${triple}"
  legacy_asset="bears-acp-adapter-${triple}"
  cargo_version=""
  if [ -f /workspace/tools/bear-armature/Cargo.toml ]; then
    cargo_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' /workspace/tools/bear-armature/Cargo.toml | head -n 1)"
  fi

  if [ "${force_compile}" = "1" ]; then
    echo "bear-armature: --force-compile set; building from local source"
    install_bear_armature_from_source "${install_dir}"
    return 0
  fi

  if [ -z "${version}" ]; then
    manifest_url="$(manifest_url_for_channel "${channel}" "${triple}")"
    manifest_tmp="$(mktemp)"
    echo "bear-armature: checking ${channel} manifest ${manifest_url}"
    if ! curl -fsSL "${manifest_url}" -o "${manifest_tmp}"; then
      legacy_manifest_url="$(legacy_manifest_url_for_channel "${channel}" "${triple}")"
      echo "bear-armature: trying legacy manifest ${legacy_manifest_url}" >&2
      curl -fsSL "${legacy_manifest_url}" -o "${manifest_tmp}" || true
    fi
    if [ -s "${manifest_tmp}" ]; then
      if mapfile -t manifest_values < <(python3 - "${manifest_tmp}" "${triple}" <<'PY'
import json
import sys

path, target = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    manifest = json.load(f)
platform = manifest.get("platforms", {}).get(target)
if not platform:
    raise SystemExit(f"manifest does not contain target {target}")
url = platform.get("binary_url")
if not url:
    raise SystemExit("manifest target does not contain binary_url")
print(manifest.get("version", ""))
print(url)
print(platform.get("sha256", ""))
print(platform.get("size", ""))
PY
      ); then
        parsed_version="${manifest_values[0]:-}"
        url="${manifest_values[1]:-}"
        sha256="${manifest_values[2]:-}"
        size="${manifest_values[3]:-}"
        echo "bear-armature: installing ${asset} version ${parsed_version:-unknown} from update manifest"
        if install_bear_armature_binary "${url}" "${install_dir}" "${sha256}" "${size}"; then
          rm -f "${manifest_tmp}"
          return 0
        fi
      else
        echo "bear-armature: could not parse update manifest ${manifest_url}" >&2
      fi
    else
      echo "bear-armature: update manifest download failed for ${manifest_url}" >&2
    fi
    rm -f "${manifest_tmp}"
    version="${cargo_version:-0.1.0}"
  fi

  url="https://github.com/bears-ai/bear-den/releases/download/bear-armature%2Fv${version}/${asset}"
  echo "bear-armature: installing ${asset} from release ${url}"
  if ! install_bear_armature_binary "${url}" "${install_dir}"; then
    legacy_url="https://github.com/bears-ai/bear-den/releases/download/bears-acp-adapter%2Fv${version}/${legacy_asset}"
    echo "bear-armature: trying legacy release ${legacy_url}" >&2
    if ! install_bear_armature_binary "${legacy_url}" "${install_dir}"; then
      echo "bear-armature: release download failed" >&2
      install_bear_armature_from_source "${install_dir}"
    fi
  fi
}

install_bear_armature

if [ -x /workspace/scripts/ensure-dev-env.sh ]; then
  /workspace/scripts/ensure-dev-env.sh
fi

if [ -x /workspace/scripts/install-git-hooks.sh ]; then
  /workspace/scripts/install-git-hooks.sh
fi

if [ -f /workspace/scripts/load-env.sh ]; then
  cat >/etc/profile.d/bears-workspace-env.sh <<'EOF'
# Load /workspace/.env for interactive shells in the devcontainer.
if [ -f /workspace/scripts/load-env.sh ]; then
  # shellcheck source=/workspace/scripts/load-env.sh
  . /workspace/scripts/load-env.sh
fi
EOF
  chmod 0644 /etc/profile.d/bears-workspace-env.sh
fi
