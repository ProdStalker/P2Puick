#!/usr/bin/env node
/**
 * Local release build without updater signing.
 * Picks a platform-native bundle target (macOS .app / Windows NSIS / Linux deb).
 *
 * On Windows, inline JSON for `--config` is mangled when spawned via `shell: true`
 * (quotes stripped). We write a temp config file and invoke the CLI via node.
 */
import { spawnSync } from "node:child_process";
import { writeFileSync, unlinkSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(join(root, "package.json"));

const bundles =
  process.platform === "darwin"
    ? "app"
    : process.platform === "win32"
      ? "nsis"
      : "deb";

const configDir = mkdtempSync(join(tmpdir(), "p2puick-tauri-try-"));
const configPath = join(configDir, "config.json");
writeFileSync(
  configPath,
  JSON.stringify({ bundle: { createUpdaterArtifacts: false } }),
);

const tauriJs = require.resolve("@tauri-apps/cli/tauri.js");

const result = spawnSync(
  process.execPath,
  [tauriJs, "build", "--bundles", bundles, "--config", configPath],
  { stdio: "inherit", cwd: root, shell: false },
);

try {
  unlinkSync(configPath);
} catch {
  // ignore
}

process.exit(result.status ?? 1);
