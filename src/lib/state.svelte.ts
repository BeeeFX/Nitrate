import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import { load, type Store } from "@tauri-apps/plugin-store";
import { DEFAULT_SETTINGS, audioCodecsFor, containersFor } from "./presets";
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
  /** Set while a native dialog is open, so the popup doesn't hide itself. */
  pinned = $state(false);
  autoStart = $state(true);
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

    try {
      this.caps = await invoke<Capabilities>("capabilities");
    } catch {
      this.caps = { hardwareEncoders: [], ffmpegFound: false };
    }

    this.ready = true;
  }

  async #restore() {
    try {
      this.#store = await load("settings.json", { autoSave: false });
      const saved = await this.#store.get<Settings>("settings");
      if (saved) this.settings = { ...DEFAULT_SETTINGS, ...saved };
      const auto = await this.#store.get<boolean>("autoStart");
      if (typeof auto === "boolean") this.autoStart = auto;
    } catch {
      // First run, or the store is unreadable — defaults are fine.
    }
  }

  async persist() {
    if (!this.#store) return;
    try {
      await this.#store.set("settings", $state.snapshot(this.settings));
      await this.#store.set("autoStart", this.autoStart);
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
      const job: Job = {
        id: `job-${nextId++}`,
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
      };
      this.jobs.push(job);

      try {
        const details = await invoke<FileInfo>("inspect_file", { path });
        job.info = details.info;
        job.name = details.name;
        job.originalBytes = details.info.sizeBytes;
        job.thumbnail = details.thumbnail
          ? convertFileSrc(details.thumbnail)
          : null;
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

  clearFinished() {
    for (const job of this.jobs) {
      if (job.status === "running" || job.status === "queued") continue;
    }
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
