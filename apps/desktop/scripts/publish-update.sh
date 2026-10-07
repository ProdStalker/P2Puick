#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

UPDATE_BASE_URL=""
VERSION=""
NOTES="Nouvelle version de P2Puick."
TARGET="universal-apple-darwin"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --url) UPDATE_BASE_URL="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --notes) NOTES="${2:-}"; shift 2 ;;
    --arm) TARGET="aarch64-apple-darwin"; shift ;;
    --intel) TARGET="x86_64-apple-darwin"; shift ;;
    --universal) TARGET="universal-apple-darwin"; shift ;;
    *) echo "unknown flag: $1" >&2; exit 1 ;;
  esac
done

[[ -n "$UPDATE_BASE_URL" ]] || { echo "--url required" >&2; exit 1; }
[[ -n "$VERSION" ]] || VERSION="$(node -p "require('./src-tauri/tauri.conf.json').version")"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS release builds must run on Darwin." >&2
  exit 1
fi

if [[ "$TARGET" == "universal-apple-darwin" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
elif [[ "$TARGET" == "aarch64-apple-darwin" ]]; then
  rustup target add aarch64-apple-darwin
else
  rustup target add x86_64-apple-darwin
fi

# Prefer key file contents (Tauri build expects the private key string).
if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" && -f "$ROOT/src-tauri/updater.key" ]]; then
  export TAURI_SIGNING_PRIVATE_KEY
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$ROOT/src-tauri/updater.key")"
fi
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
unset TAURI_SIGNING_PRIVATE_KEY_PATH

# Universal (or arch-specific) app bundle only — updater needs .app.tar.gz + .sig.
npx tauri build --target "$TARGET" --bundles app

# Collect updater artifacts.
# Workspace → repo-root/target/<triple>/… ; fallback → src-tauri/target/<triple>/…
mkdir -p dist updates
REPO_ROOT="$(cd "$ROOT/../.." && pwd)"
BUNDLE_DIRS=(
  "$REPO_ROOT/target/${TARGET}/release/bundle"
  "$ROOT/src-tauri/target/${TARGET}/release/bundle"
  # native host target (no triple dir) as last resort
  "$REPO_ROOT/target/release/bundle"
  "$ROOT/src-tauri/target/release/bundle"
)
ARCHIVE=""

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

if [[ -z "$ARCHIVE" ]]; then
  echo "Updater archive missing. Checked:" >&2
  printf '  %s\n' "${BUNDLE_DIRS[@]}" >&2
  exit 1
fi

cp "$ARCHIVE" "dist/P2Puick.app.tar.gz"
[[ -f "${ARCHIVE}.sig" ]] && cp "${ARCHIVE}.sig" "dist/P2Puick.app.tar.gz.sig"

ARCHIVE="dist/P2Puick.app.tar.gz"
SIG="${ARCHIVE}.sig"
if [[ ! -f "$ARCHIVE" || ! -f "$SIG" ]]; then
  echo "Updater archive or signature missing after copy." >&2
  exit 1
fi

# Verify universal binary when requested
APP_PATH="$(dirname "$ARCHIVE")/../macos"
# Prefer the real .app next to the tar.gz we found
FOUND_APP="$(find "$(dirname "$ARCHIVE")" -name 'P2Puick.app' -maxdepth 1 2>/dev/null | head -n1 || true)"
if [[ -z "$FOUND_APP" ]]; then
  for BUNDLE_DIR in "${BUNDLE_DIRS[@]}"; do
    if [[ -d "$BUNDLE_DIR/macos/P2Puick.app" ]]; then
      FOUND_APP="$BUNDLE_DIR/macos/P2Puick.app"
      break
    fi
  done
fi

if [[ -n "$FOUND_APP" && "$TARGET" == "universal-apple-darwin" ]]; then
  BIN_PATH="$(find "$FOUND_APP/Contents/MacOS" -type f | head -n1)"
  ARCHS="$(lipo -archs "$BIN_PATH" 2>/dev/null || true)"
  echo "lipo archs: ${ARCHS:-unknown}"
  if [[ "$ARCHS" != *"arm64"* || "$ARCHS" != *"x86_64"* ]]; then
    echo "Universal binary is missing an architecture: $ARCHS" >&2
    file "$BIN_PATH" >&2 || true
    exit 1
  fi
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
Path("updates").mkdir(exist_ok=True)
Path("updates/latest.json").write_text(
    json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
    encoding="utf-8",
)
print("Wrote updates/latest.json (darwin-aarch64 + darwin-x86_64)")
PY
