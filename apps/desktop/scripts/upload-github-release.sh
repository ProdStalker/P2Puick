#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REPO="${REPO:-ProdStalker/p2puick-updates}"
LATEST_PATH="${LATEST_PATH:-$ROOT/updates/latest.json}"
ARCHIVE_PATH="${ARCHIVE_PATH:-$ROOT/dist/P2Puick.app.tar.gz}"

[[ -f "$LATEST_PATH" ]] || { echo "latest.json missing"; exit 1; }

python3 - "$LATEST_PATH" <<'PY'
from pathlib import Path
import json, sys
path = Path(sys.argv[1])
raw = path.read_bytes()
if raw.startswith(b"\xef\xbb\xbf"):
    raw = raw[3:]
payload = json.loads(raw.decode("utf-8"))
path.write_text(json.dumps(payload, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
PY

VERSION="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).version" "$LATEST_PATH")"
TAG="v$VERSION"
TITLE="v$VERSION"
NOTES="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).notes || ''" "$LATEST_PATH")"
NOTES="${NOTES:-Nouvelle version de P2Puick.}"
CHANGELOG_PATH="$ROOT/updates/changelog.json"
PUB_DATE="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).pub_date || new Date().toISOString()" "$LATEST_PATH")"

python3 - "$REPO" "$VERSION" "$NOTES" "$PUB_DATE" "$CHANGELOG_PATH" <<'PY'
import json, subprocess, sys
from pathlib import Path
repo, version, notes, pub_date, out_path = sys.argv[1:6]
entries = {}
local = Path(out_path)
if local.is_file():
    try:
        data = json.loads(local.read_text(encoding="utf-8"))
        for e in data.get("entries", []):
            v = str(e.get("version", "")).lstrip("v")
            if v:
                entries[v] = {"version": v, "date": e.get("date", ""), "notes": (e.get("notes") or "").strip()}
    except Exception:
        pass
try:
    raw = subprocess.check_output(["gh", "api", f"repos/{repo}/releases?per_page=50"], text=True)
    for rel in json.loads(raw):
        v = str(rel.get("tag_name", "")).lstrip("v")
        body = (rel.get("body") or "").strip()
        if not v:
            continue
        entries[v] = {"version": v, "date": rel.get("published_at") or "", "notes": body or entries.get(v, {}).get("notes", "")}
except Exception:
    pass
entries[version] = {"version": version, "date": pub_date, "notes": notes.strip()}

def key(v: str):
    parts = []
    for p in v.split("."):
        try: parts.append(int(p))
        except ValueError: parts.append(0)
    while len(parts) < 3: parts.append(0)
    return tuple(parts)

ordered = sorted(entries.values(), key=lambda e: key(e["version"]), reverse=True)
Path(out_path).write_text(json.dumps({"generatedAt": pub_date, "entries": ordered}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
latest_path = Path("updates/latest.json")
latest = json.loads(latest_path.read_text(encoding="utf-8"))
blocks = [f"v{e['version']}\n{(e.get('notes') or '—').strip()}" for e in ordered[:12]]
latest["notes"] = "\n\n".join(blocks)
latest_path.write_text(json.dumps(latest, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
print(f"changelog entries: {len(ordered)}")
PY

ASSETS=("$ARCHIVE_PATH" "$LATEST_PATH" "$CHANGELOG_PATH")
NOTES_FILE="$(mktemp)"
printf '%s' "$NOTES" > "$NOTES_FILE"
trap 'rm -f "$NOTES_FILE"' EXIT

if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh release upload "$TAG" "${ASSETS[@]}" --repo "$REPO" --clobber
  gh release edit "$TAG" --repo "$REPO" --title "$TITLE" --notes-file "$NOTES_FILE"
else
  gh release create "$TAG" "${ASSETS[@]}" --repo "$REPO" --title "$TITLE" --notes-file "$NOTES_FILE"
fi

echo "Published https://github.com/$REPO/releases/tag/$TAG"
