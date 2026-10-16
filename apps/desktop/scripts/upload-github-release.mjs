#!/usr/bin/env node
/**
 * Upload dist updater archives + updates/latest.json (+ changelog) to GitHub Releases.
 */
import {
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  ROOT,
  fileExists,
  readJson,
  run,
  runCapture,
  writeJson,
} from "./lib.mjs";

const REPO = process.env.REPO || "ProdStalker/p2puick-updates";
const latestPath = process.env.LATEST_PATH || join(ROOT, "updates", "latest.json");
const distDir = join(ROOT, "dist");

if (!fileExists(latestPath)) {
  console.error("latest.json missing");
  process.exit(1);
}

// Strip BOM if present
{
  let raw = readFileSync(latestPath);
  if (raw[0] === 0xef && raw[1] === 0xbb && raw[2] === 0xbf) {
    raw = raw.subarray(3);
  }
  const payload = JSON.parse(raw.toString("utf8"));
  writeJson(latestPath, payload, { pretty: false });
}

const latest = readJson(latestPath);
const version = String(latest.version).replace(/^v/, "");
const tag = `v${version}`;
const title = tag;
let notes = (latest.notes || "").trim() || "Nouvelle version de P2Puick.";
const pubDate = latest.pub_date || new Date().toISOString();
const changelogPath = join(ROOT, "updates", "changelog.json");

const entries = new Map();
if (fileExists(changelogPath)) {
  try {
    for (const e of readJson(changelogPath).entries || []) {
      const v = String(e.version || "").replace(/^v/, "");
      if (v) {
        entries.set(v, {
          version: v,
          date: e.date || "",
          notes: (e.notes || "").trim(),
        });
      }
    }
  } catch {
    // ignore corrupt local changelog
  }
}

try {
  const raw = runCapture("gh", [
    "api",
    `repos/${REPO}/releases?per_page=50`,
  ]);
  for (const rel of JSON.parse(raw)) {
    const v = String(rel.tag_name || "").replace(/^v/, "");
    if (!v) continue;
    const body = (rel.body || "").trim();
    const prev = entries.get(v);
    entries.set(v, {
      version: v,
      date: rel.published_at || "",
      notes: body || prev?.notes || "",
    });
  }
} catch {
  // offline / no gh — keep local entries
}

entries.set(version, {
  version,
  date: pubDate,
  notes: notes.trim(),
});

function semverKey(v) {
  const parts = v.split(".").map((p) => {
    const n = Number.parseInt(p, 10);
    return Number.isNaN(n) ? 0 : n;
  });
  while (parts.length < 3) parts.push(0);
  return parts;
}

const ordered = [...entries.values()].sort((a, b) => {
  const ka = semverKey(a.version);
  const kb = semverKey(b.version);
  for (let i = 0; i < 3; i++) {
    if (ka[i] !== kb[i]) return kb[i] - ka[i];
  }
  return 0;
});

writeJson(changelogPath, { generatedAt: pubDate, entries: ordered });

const blocks = ordered
  .slice(0, 12)
  .map((e) => `v${e.version}\n${(e.notes || "—").trim()}`);
latest.notes = blocks.join("\n\n");
writeJson(latestPath, latest, { pretty: false });
notes = latest.notes;
console.log(`changelog entries: ${ordered.length}`);

const assets = [latestPath, changelogPath];
if (existsSync(distDir)) {
  for (const name of readdirSync(distDir)) {
    const full = join(distDir, name);
    if (name.endsWith(".sig")) continue;
    if (
      name.endsWith(".tar.gz") ||
      name.endsWith(".nsis.zip") ||
      name.endsWith(".zip") ||
      name.endsWith(".AppImage")
    ) {
      assets.push(full);
    }
  }
}

if (assets.length <= 2) {
  console.error("No updater archives found in dist/");
  process.exit(1);
}

const notesDir = mkdtempSync(join(tmpdir(), "p2puick-notes-"));
const notesFile = join(notesDir, "notes.txt");
writeFileSync(notesFile, notes, "utf8");

try {
  let exists = false;
  try {
    runCapture("gh", ["release", "view", tag, "--repo", REPO]);
    exists = true;
  } catch {
    exists = false;
  }

  if (exists) {
    run("gh", ["release", "upload", tag, ...assets, "--repo", REPO, "--clobber"]);
    run("gh", [
      "release",
      "edit",
      tag,
      "--repo",
      REPO,
      "--title",
      title,
      "--notes-file",
      notesFile,
    ]);
  } else {
    run("gh", [
      "release",
      "create",
      tag,
      ...assets,
      "--repo",
      REPO,
      "--title",
      title,
      "--notes-file",
      notesFile,
    ]);
  }
} finally {
  rmSync(notesDir, { recursive: true, force: true });
}

console.log(`Published https://github.com/${REPO}/releases/tag/${tag}`);
