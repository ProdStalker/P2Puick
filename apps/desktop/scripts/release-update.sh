#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REPO="${REPO:-ProdStalker/p2puick-updates}"
BUMP="patch"
VERSION=""
NOTES=""
UPLOAD=1

usage() {
  cat <<'EOF'
Usage: npm run update -- [options] [notes]

  --patch | --minor | --major | --version X.Y.Z
  --notes TEXT
  --no-upload
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --patch) BUMP="patch"; shift ;;
    --minor) BUMP="minor"; shift ;;
    --major) BUMP="major"; shift ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --notes) NOTES="${2:-}"; shift 2 ;;
    --no-upload) UPLOAD=0; shift ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "unknown flag: $1" >&2; usage >&2; exit 1 ;;
    *) NOTES="${NOTES:+$NOTES }$1"; shift ;;
  esac
done

NOTES="${NOTES:-Nouvelle version de P2Puick.}"
CURRENT="$(node -p "require('./src-tauri/tauri.conf.json').version")"
if [[ -z "$VERSION" ]]; then
  VERSION="$(python3 - "$CURRENT" "$BUMP" <<'PY'
import sys
major, minor, patch = (int(part) for part in sys.argv[1].split("."))
kind = sys.argv[2]
if kind == "major":
    major, minor, patch = major + 1, 0, 0
elif kind == "minor":
    minor, patch = minor + 1, 0
else:
    patch += 1
print(f"{major}.{minor}.{patch}")
PY
)"
fi

echo "Version $CURRENT → $VERSION"
python3 - "$VERSION" <<'PY'
import json, re, sys
from pathlib import Path
version = sys.argv[1]
package = json.loads(Path("package.json").read_text(encoding="utf-8"))
package["version"] = version
Path("package.json").write_text(json.dumps(package, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
tauri_path = Path("src-tauri/tauri.conf.json")
tauri = json.loads(tauri_path.read_text(encoding="utf-8"))
tauri["version"] = version
tauri_path.write_text(json.dumps(tauri, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
cargo_path = Path("src-tauri/Cargo.toml")
lines = cargo_path.read_text(encoding="utf-8").splitlines(keepends=True)
in_package = False
done = False
out = []
for line in lines:
    stripped = line.strip()
    if stripped == "[package]":
        in_package = True
    elif stripped.startswith("[") and stripped != "[package]":
        in_package = False
    if in_package and not done and re.match(r'^version\s*=', line):
        out.append(f'version = "{version}"\n')
        done = True
        continue
    out.append(line)
if not done:
    raise SystemExit("Could not update src-tauri/Cargo.toml package version")
cargo_path.write_text("".join(out), encoding="utf-8")
# workspace root version
root = Path("../../Cargo.toml")
if root.is_file():
    text = root.read_text(encoding="utf-8")
    text = re.sub(r'(?m)^(version\s*=\s*")[^"]+"', rf'\g<1>{version}"', text, count=1)
    # only under [workspace.package]
    pass
PY

UPDATE_BASE_URL="https://github.com/${REPO}/releases/download/v${VERSION}"
./scripts/publish-update.sh --url "$UPDATE_BASE_URL" --version "$VERSION" --notes "$NOTES"

if [[ "$UPLOAD" -eq 0 ]]; then
  echo "Stopped before GitHub upload."
  exit 0
fi

command -v gh >/dev/null
gh auth status >/dev/null
if ! gh repo view "$REPO" >/dev/null 2>&1; then
  gh repo create "$REPO" --public --description "P2Puick updater binaries" --confirm
fi
./scripts/upload-github-release.sh
echo "Done. https://github.com/${REPO}/releases/latest/download/latest.json"
