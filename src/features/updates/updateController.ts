import type { DownloadEvent } from "@tauri-apps/plugin-updater";

export interface UpdateHandle {
  version: string;
  body?: string;
  downloadAndInstall: (onEvent: (event: DownloadEvent) => void) => Promise<void>;
  close: () => Promise<void>;
}

export interface UpdatePorts {
  check: () => Promise<UpdateHandle | null>;
  relaunch: () => Promise<void>;
}

export interface UpdateState {
  phase: "idle" | "checking" | "available" | "downloading" | "installing" | "installed" | "error";
  version: string | null;
  notes: string;
  progress: number | null;
  error: string | null;
  checkedAt: number | null;
}

export const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
export const AUTO_CHECK_KEY = "providock.updates.autoCheck";

/** Native update handles remain private; checking alone never downloads or installs. */
export class UpdateController {
  state: UpdateState = {
    phase: "idle",
    version: null,
    notes: "",
    progress: null,
    error: null,
    checkedAt: null,
  };
  private handle: UpdateHandle | null = null;
  private listeners = new Set<(state: UpdateState) => void>();
  private disposed = false;

  constructor(private ports: UpdatePorts) {}

  subscribe(listener: (state: UpdateState) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private set(patch: Partial<UpdateState>) {
    if (this.disposed) return;
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((listener) => listener(this.state));
  }

  private busy() {
    return ["checking", "downloading", "installing", "installed"].includes(this.state.phase);
  }

  async check() {
    if (this.disposed || this.busy()) return;
    this.set({ phase: "checking", error: null });
    try {
      const next = await this.ports.check();
      if (this.disposed) {
        await next?.close().catch(() => {});
        return;
      }
      const old = this.handle;
      this.handle = next;
      await old?.close().catch(() => {});
      this.set({
        phase: next ? "available" : "idle",
        version: next?.version ?? null,
        notes: next?.body ?? "",
        progress: null,
        checkedAt: Date.now(),
      });
    } catch {
      this.set({ phase: "error", error: "检查失败，请检查网络后重试。", checkedAt: Date.now() });
    }
  }

  async install() {
    if (this.disposed || this.busy() || !this.handle) return;
    const update = this.handle;
    let downloaded = 0;
    let total: number | undefined;
    this.set({ phase: "downloading", progress: null, error: null });
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") total = event.data.contentLength;
        if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          this.set({ progress: total ? Math.min(100, (downloaded / total) * 100) : null });
        }
        if (event.event === "Finished") this.set({ phase: "installing", progress: 100 });
      });
    } catch {
      this.set({
        phase: "error",
        progress: null,
        error: "更新失败，可能是网络或签名校验问题，请重试。",
      });
      return;
    }
    this.handle = null;
    await update.close().catch(() => {});
    this.set({ phase: "installed", error: null, progress: 100 });
    await this.restart();
  }

  async restart() {
    if (this.state.phase !== "installed" || this.disposed) return;
    try {
      await this.ports.relaunch();
    } catch {
      this.set({ error: "更新已安装，请重启应用以完成更新。" });
    }
  }

  dispose() {
    this.disposed = true;
    this.listeners.clear();
    const handle = this.handle;
    this.handle = null;
    void handle?.close().catch(() => {});
  }
}
