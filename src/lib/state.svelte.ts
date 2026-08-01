import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as autostartIsEnabled,
} from "@tauri-apps/plugin-autostart";
import { load, type Store } from "@tauri-apps/plugin-store";
import {
  DEFAULT_SETTINGS,
  audioCodecsFor,
  containersFor,
  defaultPresetFor,
  presetsFor,
} from "./presets";
import type {
  Capabilities,
  FileInfo,
  Job,
  Plan,
  Settings,
} from "./types";

let nextId = 1;

class AppStore {
  jobs = $state<Job[]>([]);
  settings = $state<Settings>({ ...DEFAULT_SETTINGS });
  caps = $state<Capabilities | null>(null);
  /**
   * Keeps the popup open when it loses focus. On by default — a window that
   * vanishes the moment you click elsewhere is startling, and you often want
   * to watch progress while doing something else.
   */
  pinned = $state(true);
  autoStart = $state(true);
  /** Mirrors the OS setting rather than our own store, which is authoritative. */
  launchAtLogin = $state(false);
  ready = $state(false);

  #store: Store | null = null;

  active = $derived(this.jobs.filter((j) => j.status === "running").length);
  queued = $derived(this.jobs.filter((j) => j.status === "queued").length);
  busy = $derived(this.active + this.queued > 0);

  /** Aggregate progress across everything still in flight, for the header bar. */
  overall = $derived.by(() => {
    const live = this.jobs.filter(
      (j) => j.status === "running" || j.status === "queued",
    );
    if (live.length === 0) return 0;
    return live.reduce((sum, j) => sum + j.progress, 0) / live.length;
  });

  async init() {
    await this.#restore();
    await this.#wireEvents();

    // The restored pin state has to be pushed to Rust, which owns the
    // hide-on-blur behaviour.
    await invoke("set_suppress_hide", { suppress: this.pinned }).catch(() => {});

    try {
      this.launchAtLogin = await autostartIsEnabled();
    } catch {
      this.launchAtLogin = false;
    }

    try {
      this.caps = await invoke<Capabilities>("capabilities");
    } catch {
      this.caps = { hardwareEncoders: [], ffmpegFound: false };
    }

    this.ready = true;

    // Tells Rust the UI has painted, so the window can be revealed without the
    // user glimpsing an empty frame first.
    await invoke("frontend_ready").catch(() => {});

    // Anything passed on the command line — "Open with", or a file dropped
    // onto the shortcut — is waiting in Rust until now.
    await this.#drainPendingFiles();
  }

  async #drainPendingFiles() {
    try {
      const files = await invoke<string[]>("take_pending_files");
      if (files.length > 0) await this.addFiles(files);
    } catch {
      // No pending files, or the command isn't available in this context.
    }
  }

  async #restore() {
    try {
      this.#store = await load("settings.json", { autoSave: false });
      const saved = await this.#store.get<Settings>("settings");
      if (saved) this.settings = { ...DEFAULT_SETTINGS, ...saved };
      const auto = await this.#store.get<boolean>("autoStart");
      if (typeof auto === "boolean") this.autoStart = auto;
      const pinned = await this.#store.get<boolean>("pinned");
      if (typeof pinned === "boolean") this.pinned = pinned;
    } catch {
      // First run, or the store is unreadable — defaults are fine.
    }
  }

  async persist() {
    if (!this.#store) return;
    try {
      await this.#store.set("settings", $state.snapshot(this.settings));
      await this.#store.set("autoStart", this.autoStart);
      await this.#store.set("pinned", this.pinned);
      await this.#store.save();
    } catch {
      // Losing preferences isn't worth interrupting the user over.
    }
  }

  /** Applies a settings patch, repairing any codec/container combo it breaks. */
  update(patch: Partial<Settings>) {
    const next = { ...this.settings, ...patch };

    if (patch.videoCodec) {
      const allowed = containersFor(patch.videoCodec);
      if (!allowed.includes(next.container)) next.container = allowed[0];
    }
    const allowedAudio = audioCodecsFor(next.container);
    if (!allowedAudio.includes(next.audioCodec)) next.audioCodec = allowedAudio[0];

    // Presets are encoder-specific — "medium" means nothing to VP9, which wants
    // a number. Switching codec or toggling hardware has to re-base it.
    if (patch.videoCodec !== undefined || patch.hardware !== undefined) {
      const valid = presetsFor(next.videoCodec, next.hardware).map((p) => p.value);
      if (!valid.includes(next.preset)) {
        next.preset = defaultPresetFor(next.videoCodec, next.hardware);
      }
    }

    this.settings = next;
    void this.persist();
    void this.refreshPlans();
  }

  #find(id: string) {
    return this.jobs.find((j) => j.id === id);
  }

  async #wireEvents() {
    await listen<{ id: string; progress: number; stage: string }>(
      "job://progress",
      ({ payload }) => {
        const job = this.#find(payload.id);
        if (!job) return;
        if (job.status !== "running") {
          job.status = "running";
          job.startedAt ??= Date.now();
        }
        job.progress = payload.progress;
        job.stage = payload.stage;
      },
    );

    await listen<{
      id: string;
      output: string;
      finalBytes: number;
      originalBytes: number;
      attempts: number;
      notes: string[];
      width: number;
      height: number;
    }>("job://done", ({ payload }) => {
      const job = this.#find(payload.id);
      if (!job) return;
      job.status = "done";
      job.progress = 1;
      job.stage = "Done";
      job.output = payload.output;
      job.finalBytes = payload.finalBytes;
      job.originalBytes = payload.originalBytes;
      job.notes = payload.notes;
    });

    await listen<{ id: string; error: string }>(
      "job://failed",
      ({ payload }) => {
        const job = this.#find(payload.id);
        if (!job) return;
        job.status = "failed";
        job.error = payload.error;
        job.stage = "Failed";
      },
    );

    // A second launch hands its files to this instance rather than opening
    // a rival copy.
    await listen("files://open", () => void this.#drainPendingFiles());

    await listen<{ id: string }>("job://cancelled", ({ payload }) => {
      const job = this.#find(payload.id);
      if (!job) return;
      job.status = "cancelled";
      job.stage = "Cancelled";
      job.progress = 0;
    });
  }

  /** Adds dropped or picked files, probing each before it appears settled. */
  async addFiles(paths: string[]) {
    const fresh = paths.filter(
      (p) => !this.jobs.some((j) => j.path === p && j.status !== "done"),
    );

    for (const path of fresh) {
      const id = `job-${nextId++}`;
      this.jobs.push({
        id,
        path,
        name: path.split(/[\\/]/).pop() ?? path,
        info: null,
        thumbnail: null,
        status: "queued",
        progress: 0,
        stage: "Reading…",
        plan: null,
        output: null,
        finalBytes: null,
        originalBytes: null,
        error: null,
        notes: [],
        startedAt: null,
      });

      // `push` stores a reactive proxy, not the object literal above. Mutating
      // the literal would update nothing the UI can see, so everything from
      // here on has to go through the copy the array actually holds.
      const job = this.#find(id);
      if (!job) continue;

      try {
        const details = await invoke<FileInfo>("inspect_file", { path });
        job.info = details.info;
        job.name = details.name;
        job.originalBytes = details.info.sizeBytes;
        job.thumbnail = details.thumbnail;
        job.stage = "Ready";
        await this.#loadPlan(job);

        if (this.autoStart) await this.start(job.id);
      } catch (err) {
        job.status = "failed";
        job.stage = "Failed";
        job.error = String(err);
      }
    }
  }

  async #loadPlan(job: Job) {
    try {
      job.plan = await invoke<Plan>("preview_plan", {
        path: job.path,
        settings: $state.snapshot(this.settings),
      });
    } catch {
      job.plan = null;
    }
  }

  /** Re-previews every idle job after a settings change. */
  async refreshPlans() {
    await Promise.all(
      this.jobs
        .filter((j) => j.status === "queued" && j.info)
        .map((j) => this.#loadPlan(j)),
    );
  }

  async start(id: string) {
    const job = this.#find(id);
    if (!job) return;

    job.status = "queued";
    job.stage = "Waiting…";
    job.progress = 0;
    job.error = null;
    job.startedAt = null;

    try {
      await invoke("start_job", {
        id: job.id,
        path: job.path,
        settings: $state.snapshot(this.settings),
      });
    } catch (err) {
      job.status = "failed";
      job.error = String(err);
    }
  }

  async cancel(id: string) {
    await invoke("cancel_job", { id });
  }

  async startAll() {
    for (const job of this.jobs) {
      if (job.status === "queued" || job.status === "failed" || job.status === "cancelled") {
        await this.start(job.id);
      }
    }
  }

  remove(id: string) {
    const job = this.#find(id);
    if (job && (job.status === "running" || job.status === "queued")) {
      void this.cancel(id);
    }
    this.jobs = this.jobs.filter((j) => j.id !== id);
  }

  /** Drops everything that's finished, one way or another. */
  clearFinished() {
    this.jobs = this.jobs.filter(
      (j) => j.status === "running" || j.status === "queued",
    );
  }

  /** Keeps the tray popup on screen while a native dialog has focus. */
  async withDialog<T>(fn: () => Promise<T>): Promise<T> {
    await invoke("set_suppress_hide", { suppress: true });
    try {
      return await fn();
    } finally {
      // A beat of slack so the dialog's focus handoff completes first.
      setTimeout(() => {
        void invoke("set_suppress_hide", { suppress: this.pinned });
      }, 400);
    }
  }

  async setPinned(value: boolean) {
    this.pinned = value;
    await invoke("set_suppress_hide", { suppress: value });
    void this.persist();
  }

  /** The OS owns this setting, so re-read it rather than trusting our own flag. */
  async setLaunchAtLogin(value: boolean) {
    try {
      if (value) {
        await enableAutostart();
      } else {
        await disableAutostart();
      }
      this.launchAtLogin = await autostartIsEnabled();
    } catch {
      this.launchAtLogin = false;
    }
  }
}

export const app = new AppStore();

/** Seconds remaining, or null when there isn't enough signal yet. */
export function etaFor(job: Job): number | null {
  if (job.status !== "running" || !job.startedAt || job.progress < 0.03) {
    return null;
  }
  const elapsed = (Date.now() - job.startedAt) / 1000;
  return elapsed / job.progress - elapsed;
}
