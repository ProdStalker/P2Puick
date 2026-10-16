#!/usr/bin/env node
/**
 * Bump version, build signed updater artifacts for this OS, optionally upload to GitHub.
 */
import { join } from "node:path";
import {
  ROOT,
  REPO_ROOT,
  bumpSemver,
  parseArgs,
  readJson,
  run,
  runCapture,
  updateTomlPackageVersion,
  writeJson,
} from "./lib.mjs";

const REPO = process.env.REPO || "ProdStalker/p2puick-updates";

let bump = "patch";
let version = "";
let notes = "";
let upload = true;

function usage() {
  console.log(`Usage: npm run update -- [options] [notes]

  --patch | --minor | --major | --version X.Y.Z
  --notes TEXT
  --no-upload
`);
}

const positional = parseArgs(process.argv.slice(2), {
  help: usage,
  "--patch": () => {
    bump = "patch";
  },
  "--minor": () => {
    bump = "minor";
  },
  "--major": () => {
    bump = "major";
  },
  "--version": (v) => {
    version = v;
  },
  "--notes": (v) => {
    notes = v;
  },
  "--no-upload": () => {
    upload = false;
  },
});

if (positional.length) {
  notes = notes ? `${notes} ${positional.join(" ")}` : positional.join(" ");
}
notes ||= "Nouvelle version de P2Puick.";

const current = readJson(join(ROOT, "src-tauri", "tauri.conf.json")).version;
if (!version) version = bumpSemver(current, bump);

console.log(`Version ${current} → ${version}`);

const packageJson = readJson(join(ROOT, "package.json"));
packageJson.version = version;
writeJson(join(ROOT, "package.json"), packageJson);

const tauriConf = readJson(join(ROOT, "src-tauri", "tauri.conf.json"));
tauriConf.version = version;
writeJson(join(ROOT, "src-tauri", "tauri.conf.json"), tauriConf);

updateTomlPackageVersion(
  join(ROOT, "src-tauri", "Cargo.toml"),
  version,
  "[package]",
);
updateTomlPackageVersion(
  join(REPO_ROOT, "Cargo.toml"),
  version,
  "[workspace.package]",
);

const updateBaseUrl = `https://github.com/${REPO}/releases/download/v${version}`;
run(
  "node",
  [
    "./scripts/publish-update.mjs",
    "--url",
    updateBaseUrl,
    "--version",
    version,
  ],
  { env: { ...process.env, P2PUICK_UPDATE_NOTES: notes } },
);

if (!upload) {
  console.log("Stopped before GitHub upload.");
  process.exit(0);
}

run("gh", ["auth", "status"]);
try {
  runCapture("gh", ["repo", "view", REPO]);
} catch {
  run("gh", [
    "repo",
    "create",
    REPO,
    "--public",
    "--description",
    "P2Puick updater binaries",
    "--confirm",
  ]);
}

run("node", ["./scripts/upload-github-release.mjs"]);
console.log(
  `Done. https://github.com/${REPO}/releases/latest/download/latest.json`,
);
