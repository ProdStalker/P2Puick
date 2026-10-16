import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
  copyFileSync,
  statSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const REPO_ROOT = resolve(ROOT, "../..");

export function run(cmd, args, opts = {}) {
  const result = spawnSync(cmd, args, {
    stdio: "inherit",
    // Windows needs shell to resolve .cmd shims (npx, gh); keep args unquoted via array form.
    shell: process.platform === "win32",
    cwd: ROOT,
    ...opts,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
  return result;
}

export function runCapture(cmd, args, opts = {}) {
  const result = spawnSync(cmd, args, {
    encoding: "utf8",
    shell: process.platform === "win32",
    cwd: ROOT,
    ...opts,
  });
  if (result.status !== 0) {
    const err = (result.stderr || result.stdout || "").trim();
    throw new Error(err || `${cmd} failed`);
  }
  return (result.stdout || "").trim();
}

export function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

export function writeJson(path, value, { pretty = true } = {}) {
  const text = pretty
    ? `${JSON.stringify(value, null, 2)}\n`
    : JSON.stringify(value);
  writeFileSync(path, text, "utf8");
}

export function ensureDir(path) {
  mkdirSync(path, { recursive: true });
}

export function bumpSemver(version, kind) {
  let [major, minor, patch] = version.split(".").map((p) => Number.parseInt(p, 10));
  if ([major, minor, patch].some((n) => Number.isNaN(n))) {
    throw new Error(`Invalid version: ${version}`);
  }
  if (kind === "major") {
    major += 1;
    minor = 0;
    patch = 0;
  } else if (kind === "minor") {
    minor += 1;
    patch = 0;
  } else {
    patch += 1;
  }
  return `${major}.${minor}.${patch}`;
}

/** Update version = "…" under the first [package] or [workspace.package] section. */
export function updateTomlPackageVersion(path, version, sectionHeader) {
  const lines = readFileSync(path, "utf8").split(/(?<=\n)/);
  let inSection = false;
  let done = false;
  const out = [];
  for (const line of lines) {
    const stripped = line.trim();
    if (stripped === sectionHeader) {
      inSection = true;
    } else if (stripped.startsWith("[") && stripped !== sectionHeader) {
      inSection = false;
    }
    if (inSection && !done && /^\s*version\s*=/.test(line)) {
      out.push(`version = "${version}"\n`);
      done = true;
      continue;
    }
    out.push(line);
  }
  if (!done) {
    throw new Error(`Could not update ${path} (${sectionHeader} version)`);
  }
  writeFileSync(path, out.join(""), "utf8");
}

export function loadSigningKey() {
  if (process.env.TAURI_SIGNING_PRIVATE_KEY) return;
  const keyPath = join(ROOT, "src-tauri", "updater.key");
  if (!existsSync(keyPath)) {
    throw new Error(
      "Missing TAURI_SIGNING_PRIVATE_KEY and src-tauri/updater.key",
    );
  }
  process.env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(keyPath, "utf8").trim();
  process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";
  delete process.env.TAURI_SIGNING_PRIVATE_KEY_PATH;
}

export function findFirstFile(dirs, predicate) {
  for (const dir of dirs) {
    if (!existsSync(dir)) continue;
    const entries = readdirSync(dir);
    for (const name of entries) {
      const full = join(dir, name);
      if (predicate(name, full)) return full;
    }
  }
  return null;
}

export function listBundleRoots(targetTriple) {
  const roots = [];
  if (targetTriple) {
    roots.push(
      join(REPO_ROOT, "target", targetTriple, "release", "bundle"),
      join(ROOT, "src-tauri", "target", targetTriple, "release", "bundle"),
    );
  }
  roots.push(
    join(REPO_ROOT, "target", "release", "bundle"),
    join(ROOT, "src-tauri", "target", "release", "bundle"),
  );
  return roots;
}

export function copyArtifact(src, dest) {
  ensureDir(dirname(dest));
  copyFileSync(src, dest);
  const sig = `${src}.sig`;
  if (existsSync(sig)) {
    copyFileSync(sig, `${dest}.sig`);
  }
}

export function fileExists(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

export function parseArgs(argv, handlers) {
  const rest = [];
  let i = 0;
  while (i < argv.length) {
    const arg = argv[i];
    if (arg === "-h" || arg === "--help") {
      handlers.help?.();
      process.exit(0);
    }
    if (arg.startsWith("-")) {
      const fn = handlers[arg];
      if (!fn) {
        console.error(`unknown flag: ${arg}`);
        handlers.help?.();
        process.exit(1);
      }
      const needValue = fn.length >= 1;
      if (needValue) {
        const value = argv[i + 1];
        if (value == null || value.startsWith("-")) {
          console.error(`missing value for ${arg}`);
          process.exit(1);
        }
        fn(value);
        i += 2;
      } else {
        fn();
        i += 1;
      }
    } else {
      rest.push(arg);
      i += 1;
    }
  }
  return rest;
}
