import { CommonModule } from "@angular/common";
import { Component, OnDestroy, OnInit, signal } from "@angular/core";
import { FormsModule } from "@angular/forms";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  checkManually,
  checkOnStartup,
  loadAppVersion,
} from "./updates/updates";
import {
  fetchChangelog,
  formatDateShort,
  type ChangelogEntry,
} from "./updates/changelog";

type Mode = "home" | "host" | "join" | "transfer" | "history";

type ProgressEvent = {
  kind: string;
  relativePath: string;
  bytesDone: number;
  bytesTotal: number;
  filesDone: number;
  filesTotal: number;
  message: string;
};

type FailedEntry = {
  relativePath: string;
  absolutePath: string;
  size: number;
  error: string;
  failedAtUnix: number;
};

@Component({
  selector: "app-root",
  imports: [CommonModule, FormsModule],
  templateUrl: "./app.component.html",
  styleUrl: "./app.component.css",
})
export class AppComponent implements OnInit, OnDestroy {
  mode = signal<Mode>("home");
  version = signal("");
  status = signal("Prêt");
  pairingCode = signal("");
  hostPort = signal(47821);
  lanAddresses = signal<string[]>([]);
  joinCode = "";
  manualHost = "";
  manualPort = 47821;
  selectedPaths = signal<string[]>([]);
  /** One directory name per line — skipped while walking trees. */
  excludeText = "";
  destDir = signal("");
  peerAddr = signal("");
  progress = signal<ProgressEvent | null>(null);
  busy = signal(false);
  error = signal("");
  changelog = signal<ChangelogEntry[]>([]);
  retryQueue = signal<FailedEntry[]>([]);
  retryQueuePath = signal("");

  private unlistenProgress?: UnlistenFn;
  private unlistenStatus?: UnlistenFn;

  async ngOnInit(): Promise<void> {
    this.version.set(await loadAppVersion());
    try {
      const excludes = await invoke<string[]>("default_excludes");
      this.excludeText = (excludes ?? []).join("\n");
    } catch {
      this.excludeText = "node_modules\nvendor\nvar\n.git\ntarget\ndist";
    }
    await this.refreshRetryQueue();
    void checkOnStartup();
    this.unlistenProgress = await listen<ProgressEvent>("transfer-progress", (event) => {
      this.progress.set(event.payload);
      this.status.set(event.payload.message || event.payload.kind);
      if (event.payload.kind === "complete") {
        this.busy.set(false);
        void this.refreshRetryQueue();
      }
      if (event.payload.kind === "fileFailed") {
        void this.refreshRetryQueue();
      }
      if (event.payload.kind === "error" || event.payload.kind === "cancelled") {
        this.busy.set(false);
        this.error.set(event.payload.message);
        void this.refreshRetryQueue();
      }
    });
    this.unlistenStatus = await listen<Record<string, unknown>>("session-status", (event) => {
      const payload = event.payload;
      if (typeof payload["pairingCode"] === "string") {
        this.pairingCode.set(payload["pairingCode"]);
      }
      if (typeof payload["addr"] === "string") {
        this.peerAddr.set(payload["addr"]);
      }
      if (typeof payload["status"] === "string") {
        this.status.set(String(payload["status"]));
      }
    });
  }

  ngOnDestroy(): void {
    this.unlistenProgress?.();
    this.unlistenStatus?.();
  }

  async goHome(): Promise<void> {
    await this.cancel();
    try {
      await invoke("stop_host");
    } catch {
      // ignore
    }
    this.mode.set("home");
    this.error.set("");
    this.progress.set(null);
    this.busy.set(false);
  }

  async startHost(): Promise<void> {
    this.error.set("");
    this.busy.set(true);
    try {
      const res = await invoke<{
        pairingCode: string;
        port: number;
        addresses: string[];
      }>("start_host", {
        pairingCode: null,
        port: null,
      });
      this.pairingCode.set(res.pairingCode);
      this.hostPort.set(res.port);
      this.lanAddresses.set(res.addresses ?? []);
      this.mode.set("host");
      this.status.set(`Hôte actif — code ${res.pairingCode}`);
    } catch (e) {
      this.error.set(String(e));
    } finally {
      this.busy.set(false);
    }
  }

  async stopHost(): Promise<void> {
    await this.cancel();
    try {
      await invoke("stop_host");
    } catch {
      // ignore
    }
    this.mode.set("home");
    this.error.set("");
    this.progress.set(null);
    this.busy.set(false);
  }

  goJoin(): void {
    this.mode.set("join");
    this.error.set("");
  }

  async pickSources(): Promise<void> {
    try {
      const files = await invoke<string[]>("pick_files");
      if (files?.length) {
        this.selectedPaths.set(files);
      }
    } catch (e) {
      this.error.set(String(e));
    }
  }

  async pickSourceFolder(): Promise<void> {
    try {
      const folder = await invoke<string | null>("pick_folder", {
        title: "Choisir un dossier à envoyer",
      });
      if (folder) {
        // Replace selection (don't accumulate folders).
        this.selectedPaths.set([folder]);
      }
    } catch (e) {
      this.error.set(String(e));
    }
  }

  clearSources(): void {
    this.selectedPaths.set([]);
  }

  async pickDest(): Promise<void> {
    try {
      const folder = await invoke<string | null>("pick_folder", {
        title: "Dossier de destination",
      });
      if (folder) {
        this.destDir.set(folder);
      }
    } catch (e) {
      this.error.set(String(e));
    }
  }

  async refreshRetryQueue(): Promise<void> {
    try {
      const entries = await invoke<FailedEntry[]>("list_retry_queue");
      this.retryQueue.set(entries ?? []);
      const path = await invoke<string>("retry_queue_file_path");
      this.retryQueuePath.set(path ?? "");
    } catch {
      this.retryQueue.set([]);
    }
  }

  async clearRetryQueue(): Promise<void> {
    try {
      await invoke("clear_retry_queue");
      await this.refreshRetryQueue();
    } catch (e) {
      this.error.set(String(e));
    }
  }

  async removeRetryEntry(absolutePath: string): Promise<void> {
    try {
      await invoke("remove_retry_entry", { absolutePath });
      await this.refreshRetryQueue();
    } catch (e) {
      this.error.set(String(e));
    }
  }

  async launchSend(): Promise<void> {
    if (!this.selectedPaths().length) {
      this.error.set("Sélectionne au moins un fichier ou dossier.");
      return;
    }
    this.error.set("");
    this.busy.set(true);
    this.mode.set("transfer");
    this.status.set("En attente du pair… Lance le transfert côté hôte puis rejoins depuis l’autre PC.");
    const excludeDirNames = this.excludeText
      .split(/[\n,]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    try {
      await invoke("begin_send", {
        paths: this.selectedPaths(),
        excludeDirNames,
      });
      this.status.set("Transfert terminé");
    } catch (e) {
      this.error.set(String(e));
    } finally {
      this.busy.set(false);
      await this.refreshRetryQueue();
    }
  }

  async launchRetrySend(): Promise<void> {
    if (!this.retryQueue().length) {
      this.error.set("Aucun fichier en échec à renvoyer.");
      return;
    }
    this.error.set("");
    this.busy.set(true);
    this.mode.set("transfer");
    this.status.set("Renvoi des fichiers en échec — l’autre PC doit rejoindre la session.");
    try {
      await invoke("begin_send_retry");
      this.status.set("Renvoi terminé");
    } catch (e) {
      this.error.set(String(e));
    } finally {
      this.busy.set(false);
      await this.refreshRetryQueue();
    }
  }

  async joinAndReceive(): Promise<void> {
    this.error.set("");
    if (!/^\d{6}$/.test(this.joinCode.trim())) {
      this.error.set("Le code doit contenir 6 chiffres.");
      return;
    }
    if (!this.destDir()) {
      this.error.set("Choisis un dossier de destination.");
      return;
    }
    this.busy.set(true);
    this.mode.set("transfer");
    try {
      const addr = await invoke<string>("join_session", {
        pairingCode: this.joinCode.trim(),
        host: this.manualHost.trim() || null,
        port: this.manualPort || null,
      });
      this.peerAddr.set(addr);
      this.status.set(`Connecté à ${addr}`);
      await invoke("begin_receive", { destDir: this.destDir() });
      this.status.set("Réception terminée");
    } catch (e) {
      this.error.set(String(e));
    } finally {
      this.busy.set(false);
    }
  }

  async cancel(): Promise<void> {
    try {
      await invoke("cancel_transfer");
    } catch {
      // ignore
    }
    this.busy.set(false);
    this.status.set("Annulé");
  }

  async openUpdates(): Promise<void> {
    await checkManually();
  }

  async openHistory(): Promise<void> {
    this.mode.set("history");
    try {
      const data = await fetchChangelog();
      this.changelog.set(data.entries);
    } catch (e) {
      this.error.set(String(e));
      this.changelog.set([]);
    }
  }

  formatDate(iso: string): string {
    return formatDateShort(iso);
  }

  progressPercent(): number {
    const p = this.progress();
    if (!p || !p.bytesTotal) return 0;
    return Math.min(100, Math.round((p.bytesDone / p.bytesTotal) * 100));
  }
}
