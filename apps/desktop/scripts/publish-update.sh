#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

UPDATE_BASE_URL=""
VERSION=""
NOTES="Nouvelle version de P2Puick."

while [[ $# -gt 0 ]]; do
  case "$1" in
    --url) UPDATE_BASE_URL="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --notes) NOTES="${2:-}"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 1 ;;
  esac
done

[[ -n "$UPDATE_BASE_URL" ]] || { echo "--url required" >&2; exit 1; }
[[ -n "$VERSION" ]] || VERSION="$(node -p "require('./src-tauri/tauri.conf.json').version")"

# Prefer key file contents (Tauri build expects the private key string).
if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" && -f "$ROOT/src-tauri/updater.key" ]]; then
  export TAURI_SIGNING_PRIVATE_KEY
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$ROOT/src-tauri/updater.key")"
fi
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
unset TAURI_SIGNING_PRIVATE_KEY_PATH

# App bundle only (skip DMG — updater needs .app.tar.gz + .sig).
npx tauri build --bundles app

# Collect updater artifacts produced by Tauri.
# Cargo workspace → repo-root target/; fallback → src-tauri/target/.
mkdir -p dist updates
REPO_ROOT="$(cd "$ROOT/../.." && pwd)"
BUNDLE_DIRS=(
  "$REPO_ROOT/target/release/bundle"
  "$ROOT/src-tauri/target/release/bundle"
)
ARCHIVE=""
SIG=""

for BUNDLE_DIR in "${BUNDLE_DIRS[@]}"; do
  if [[ -f "$BUNDLE_DIR/macos/P2Puick.app.tar.gz" ]]; then
    ARCHIVE="$BUNDLE_DIR/macos/P2Puick.app.tar.gz"
    break
  fi
  if ls "$BUNDLE_DIR"/macos/*.app.tar.gz >/dev/null 2>&1; then
    ARCHIVE="$(ls "$BUNDLE_DIR"/macos/*.app.tar.gz | head -n1)"
    break
  fi
done

if [[ -n "$ARCHIVE" ]]; then
  cp "$ARCHIVE" "dist/P2Puick.app.tar.gz"
  [[ -f "${ARCHIVE}.sig" ]] && cp "${ARCHIVE}.sig" "dist/P2Puick.app.tar.gz.sig"
fi

ARCHIVE="dist/P2Puick.app.tar.gz"
SIG="${ARCHIVE}.sig"
if [[ ! -f "$ARCHIVE" || ! -f "$SIG" ]]; then
  echo "Updater archive or signature missing. Checked:" >&2
  printf '  %s\n' "${BUNDLE_DIRS[@]}" >&2
  for BUNDLE_DIR in "${BUNDLE_DIRS[@]}"; do
    find "$BUNDLE_DIR" -name '*.sig' 2>/dev/null | head
  done
  exit 1
fi

SIGNATURE="$(tr -d '\r\n' < "$SIG")"
ARCHIVE_URL="${UPDATE_BASE_URL%/}/P2Puick.app.tar.gz"
PUB_DATE="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

python3 - "$VERSION" "$NOTES" "$PUB_DATE" "$SIGNATURE" "$ARCHIVE_URL" <<'PY'
import json, sys
from pathlib import Path
version, notes, pub_date, signature, url = sys.argv[1:6]
payload = {
    "version": version,
    "notes": notes,
    "pub_date": pub_date,
    "platforms": {
        "darwin-aarch64": {"signature": signature, "url": url},
        "darwin-x86_64": {"signature": signature, "url": url},
    },
}
# Placeholder entries for Win/Linux; filled when built on those hosts.
Path("updates").mkdir(exist_ok=True)
Path("updates/latest.json").write_text(
    json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
    encoding="utf-8",
)
print("Wrote updates/latest.json")
PY
