#!/usr/bin/env node
/**
 * Build signed updater artifacts for the current OS and write updates/latest.json.
 * macOS  → universal .app.tar.gz (darwin-aarch64 + darwin-x86_64)
 * Windows → NSIS .nsis.zip (windows-x86_64)
 * Linux  → AppImage updater tarball (linux-x86_64)
 */
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import {
  ROOT,
  copyArtifact,
  ensureDir,
  fileExists,
  findFirstFile,
  listBundleRoots,
  loadSigningKey,
  parseArgs,
  readJson,
  run,
  writeJson,
} from "./lib.mjs";

let updateBaseUrl = "";
let version = "";
let notes =
  process.env.P2PUICK_UPDATE_NOTES || "Nouvelle version de P2Puick.";
let macTarget = "universal-apple-darwin";

function usage() {
  console.log(`Usage: npm run publish:update -- --url URL --version X.Y.Z [options]

  --url URL
  --version X.Y.Z
  --notes TEXT
  --arm | --intel | --universal   (macOS only)
`);
}

parseArgs(process.argv.slice(2), {
  help: usage,
  "--url": (v) => {
    updateBaseUrl = v;
  },
  "--version": (v) => {
    version = v;
  },
  "--notes": (v) => {
    notes = v;
  },
  "--arm": () => {
    macTarget = "aarch64-apple-darwin";
  },
  "--intel": () => {
    macTarget = "x86_64-apple-darwin";
  },
  "--universal": () => {
    macTarget = "universal-apple-darwin";
  },
});

if (!updateBaseUrl) {
  console.error("--url required");
  process.exit(1);
}
if (!version) {
  version = readJson(join(ROOT, "src-tauri", "tauri.conf.json")).version;
}

loadSigningKey();
ensureDir(join(ROOT, "dist"));
ensureDir(join(ROOT, "updates"));

const platform = process.platform;
let archiveName;
let platformKeys;
let targetTriple = null;

if (platform === "darwin") {
  archiveName = "P2Puick.app.tar.gz";
  platformKeys = ["darwin-aarch64", "darwin-x86_64"];
  targetTriple = macTarget;
  if (macTarget === "universal-apple-darwin") {
    run("rustup", ["target", "add", "aarch64-apple-darwin", "x86_64-apple-darwin"]);
  } else {
    run("rustup", ["target", "add", macTarget]);
  }
  run("npx", ["tauri", "build", "--target", macTarget, "--bundles", "app"]);
} else if (platform === "win32") {
  archiveName = "P2Puick-setup.nsis.zip";
  platformKeys = ["windows-x86_64"];
  run("npx", ["tauri", "build", "--bundles", "nsis"]);
} else {
  archiveName = "P2Puick.AppImage.tar.gz";
  platformKeys = ["linux-x86_64"];
  run("npx", ["tauri", "build", "--bundles", "appimage"]);
}

const bundleRoots = listBundleRoots(targetTriple);
let archiveSrc = null;

if (platform === "darwin") {
  for (const root of bundleRoots) {
    const macosDir = join(root, "macos");
    const preferred = join(macosDir, "P2Puick.app.tar.gz");
    if (fileExists(preferred)) {
      archiveSrc = preferred;
      break;
    }
    const found = findFirstFile([macosDir], (name) => name.endsWith(".app.tar.gz"));
    if (found) {
      archiveSrc = found;
      break;
    }
  }
} else if (platform === "win32") {
  for (const root of bundleRoots) {
    const found = findFirstFile(
      [join(root, "nsis")],
      (name) => name.endsWith(".nsis.zip") || name.endsWith("-setup.zip"),
    );
    if (found) {
      archiveSrc = found;
      break;
    }
  }
} else {
  for (const root of bundleRoots) {
    const dir = join(root, "appimage");
    const updaterTar = findFirstFile([dir], (name) =>
      name.endsWith(".appimage.tar.gz"),
    );
    if (updaterTar) {
      archiveSrc = updaterTar;
      break;
    }
  }
}

if (!archiveSrc) {
  console.error("Updater archive missing. Checked:");
  for (const root of bundleRoots) console.error(`  ${root}`);
  process.exit(1);
}

const archiveDest = join(ROOT, "dist", archiveName);
copyArtifact(archiveSrc, archiveDest);
const sigPath = `${archiveDest}.sig`;
if (!fileExists(archiveDest) || !fileExists(sigPath)) {
  console.error("Updater archive or signature missing after copy.");
  process.exit(1);
}

if (platform === "darwin" && macTarget === "universal-apple-darwin") {
  let appPath = null;
  for (const root of bundleRoots) {
    const candidate = join(root, "macos", "P2Puick.app");
    if (existsSync(candidate)) {
      appPath = candidate;
      break;
    }
  }
  if (appPath) {
    const binDir = join(appPath, "Contents", "MacOS");
    const bins = existsSync(binDir) ? readdirSync(binDir) : [];
    if (bins[0]) {
      const lipo = spawnSync("lipo", ["-archs", join(binDir, bins[0])], {
        encoding: "utf8",
      });
      const archs = (lipo.stdout || "").trim();
      console.log(`lipo archs: ${archs || "unknown"}`);
      if (!archs.includes("arm64") || !archs.includes("x86_64")) {
        console.error(`Universal binary is missing an architecture: ${archs}`);
        process.exit(1);
      }
    }
  }
}

const signature = readFileSync(sigPath, "utf8").replace(/\r?\n/g, "");
const archiveUrl = `${updateBaseUrl.replace(/\/$/, "")}/${archiveName}`;
const pubDate = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");

const latestPath = join(ROOT, "updates", "latest.json");
let platforms = {};
if (fileExists(latestPath)) {
  try {
    platforms = { ...readJson(latestPath).platforms };
  } catch {
    platforms = {};
  }
}
for (const key of platformKeys) {
  platforms[key] = { signature, url: archiveUrl };
}

writeJson(
  latestPath,
  { version, notes, pub_date: pubDate, platforms },
  { pretty: false },
);
console.log(
  `Wrote updates/latest.json (${platformKeys.join(" + ")}) → ${archiveName}`,
);
