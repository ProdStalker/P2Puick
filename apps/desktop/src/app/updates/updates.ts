import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
  entriesSince,
  fetchChangelog,
  formatEntriesPlain,
  type ChangelogEntry,
} from "./changelog";

const TITLE = "Mise à jour";

let checking = false;

export async function loadAppVersion(): Promise<string> {
  if (!isTauri()) return "0.1.0";
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

export async function checkOnStartup(): Promise<void> {
  if (!isTauri() || checking) return;
  try {
    const update = await check();
    if (!update) return;
    const install = await ask(await availableMessage(update), {
      title: TITLE,
      kind: "info",
      okLabel: "Installer",
      cancelLabel: "Plus tard",
    });
    if (install) {
      await downloadAndInstall(update);
    }
  } catch {
    // Silent on startup
  }
}

export async function checkManually(): Promise<void> {
  if (!isTauri()) {
    await message("Les mises à jour ne sont disponibles que dans l’app desktop.", {
      title: TITLE,
      kind: "info",
    });
    return;
  }
  if (checking) return;
  checking = true;
  try {
    const update = await check();
    if (!update) {
      await message("Tu as déjà la dernière version.", { title: TITLE, kind: "info" });
      return;
    }
    const install = await ask(await availableMessage(update), {
      title: TITLE,
      kind: "info",
      okLabel: "Installer",
      cancelLabel: "Annuler",
    });
    if (install) {
      await downloadAndInstall(update);
    }
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    await message(`Impossible de vérifier les mises à jour.\n\n${detail}`, {
      title: TITLE,
      kind: "error",
    });
  } finally {
    checking = false;
  }
}

async function availableMessage(update: Update): Promise<string> {
  const current = await getVersion().catch(() => "");
  const history = await loadUpdateHistory(current, update.version);
  if (history) {
    const header =
      history.length > 1
        ? `La version ${update.version} est disponible (${history.length} versions depuis la tienne).\n\n`
        : `La version ${update.version} est disponible.\n\n`;
    return `${header}${formatEntriesPlain(history)}`;
  }
  const notes = update.body?.trim();
  return notes
    ? `La version ${update.version} est disponible.\n\n${notes}`
    : `La version ${update.version} est disponible.`;
}

async function loadUpdateHistory(
  fromVersion: string,
  toVersion: string,
): Promise<ChangelogEntry[] | null> {
  if (!fromVersion) return null;
  try {
    const changelog = await fetchChangelog();
    const entries = entriesSince(changelog, fromVersion, toVersion);
    return entries.length ? entries : null;
  } catch {
    return null;
  }
}

async function downloadAndInstall(update: Update): Promise<void> {
  await update.downloadAndInstall();
  const restart = await ask("Mise à jour installée. Redémarrer maintenant ?", {
    title: TITLE,
    kind: "info",
    okLabel: "Redémarrer",
    cancelLabel: "Plus tard",
  });
  if (restart) {
    await relaunch();
  }
}
