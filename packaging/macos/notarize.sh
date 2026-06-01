#!/bin/sh
set -eu

usage() {
  cat <<'USAGE'
Usage: notarize.sh (--pkg <path> | --dmg <path>)

Notarize and staple a signed macOS package or DMG with Apple's notary service.

Required environment:
  APP_STORE_CONNECT_API_KEY_ID
  APP_STORE_CONNECT_API_ISSUER_ID
  APP_STORE_CONNECT_API_KEY_PATH

The API key path should point to the .p8 private key file.
USAGE
}

pkg=""
dmg=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --pkg)
      pkg="${2:-}"
      shift 2
      ;;
    --dmg)
      dmg="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "notarize.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

artifact=""
artifact_kind=""
assessment_type=""

if [ -n "$pkg" ] && [ -n "$dmg" ]; then
  echo "notarize.sh: pass only one of --pkg or --dmg" >&2
  exit 2
fi

if [ -n "$pkg" ]; then
  artifact="$pkg"
  artifact_kind="package"
  assessment_type="install"
elif [ -n "$dmg" ]; then
  artifact="$dmg"
  artifact_kind="dmg"
  assessment_type="open"
else
  echo "notarize.sh: one of --pkg or --dmg is required" >&2
  exit 2
fi

if [ ! -f "$artifact" ]; then
  echo "notarize.sh: $artifact_kind not found: $artifact" >&2
  exit 2
fi

: "${APP_STORE_CONNECT_API_KEY_ID:?APP_STORE_CONNECT_API_KEY_ID is required}"
: "${APP_STORE_CONNECT_API_ISSUER_ID:?APP_STORE_CONNECT_API_ISSUER_ID is required}"
: "${APP_STORE_CONNECT_API_KEY_PATH:?APP_STORE_CONNECT_API_KEY_PATH is required}"

xcrun notarytool submit "$artifact" \
  --key "$APP_STORE_CONNECT_API_KEY_PATH" \
  --key-id "$APP_STORE_CONNECT_API_KEY_ID" \
  --issuer "$APP_STORE_CONNECT_API_ISSUER_ID" \
  --wait

xcrun stapler staple "$artifact"
xcrun stapler validate "$artifact"

spctl --assess --type "$assessment_type" --verbose=4 "$artifact"

echo "notarize.sh: notarized and stapled $artifact_kind $artifact"
