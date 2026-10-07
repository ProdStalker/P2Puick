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
  joinCode = "";
  manualHost = "";
  manualPort = 47821;
  selectedPaths = signal<string[]>([]);
  destDir = signal("");
  peerAddr = signal("");
  progress = signal<ProgressEvent | null>(null);
  busy = signal(false);
  error = signal("");
  changelog = signal<ChangelogEntry[]>([]);

  private unlistenProgress?: UnlistenFn;
  private unlistenStatus?: UnlistenFn;

  async ngOnInit(): Promise<void> {
    this.version.set(await loadAppVersion());
    void checkOnStartup();
    this.unlistenProgress = await listen<ProgressEvent>("transfer-progress", (event) => {
      this.progress.set(event.payload);
      this.status.set(event.payload.message || event.payload.kind);
      if (event.payload.kind === "complete") {
        this.busy.set(false);
      }
      if (event.payload.kind === "error" || event.payload.kind === "cancelled") {
        this.busy.set(false);
        this.error.set(event.payload.message);
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

  goHome(): void {
    this.mode.set("home");
    this.error.set("");
  }

  async startHost(): Promise<void> {
    this.error.set("");
    this.busy.set(true);
    try {
      const res = await invoke<{ pairingCode: string; port: number }>("start_host", {
        pairingCode: null,
        port: null,
      });
      this.pairingCode.set(res.pairingCode);
      this.mode.set("host");
      this.status.set(`Hôte actif — code ${res.pairingCode}`);
    } catch (e) {
      this.error.set(String(e));
    } finally {
      this.busy.set(false);
    }
  }

  async stopHost(): Promise<void> {
    try {
      await invoke("stop_host");
    } catch {
      // ignore
    }
    this.goHome();
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
        this.selectedPaths.set([...this.selectedPaths(), folder]);
      }
    } catch (e) {
      this.error.set(String(e));
    }
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

  async launchSend(): Promise<void> {
    if (!this.selectedPaths().length) {
      this.error.set("Sélectionne au moins un fichier ou dossier.");
      return;
    }
    this.error.set("");
    this.busy.set(true);
    this.mode.set("transfer");
    this.status.set("En attente du pair… Lance le transfert côté hôte puis rejoins depuis l’autre PC.");
    try {
      await invoke("begin_send", { paths: this.selectedPaths() });
      this.status.set("Transfert terminé");
    } catch (e) {
      this.error.set(String(e));
    } finally {
      this.busy.set(false);
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
