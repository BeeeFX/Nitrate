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
  LONG_VIDEO_SECONDS,
  audioCodecsFor,
  containersFor,
  defaultPresetFor,
  isVideoFile,
  presetsFor,
} from "./presets";
import {
  emptyEdits,
  type Capabilities,
  type Edits,
  type FileInfo,
  type Job,
  type Plan,
  type Settings,
  type UrlInfo,
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
  /** Id of the job whose editor is open, if any. */
  editingId = $state<string | null>(null);
  /** Transient message, for things the user should know but needn't act on. */
  notice = $state<string | null>(null);

  #store: Store | null = null;

  active = $derived(this.jobs.filter((j) => j.status === "running").length);
  queued = $derived(this.jobs.filter((j) => j.status === "queued").length);
  // A held job isn't working and isn't finished — it's waiting for a decision.
  held = $derived(this.jobs.filter((j) => j.status === "held").length);
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
      passedThrough: boolean;
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
      job.passedThrough = payload.passedThrough;

      // A link had no local file until now. Point the job at what landed, so
      // editing and re-compressing work on something that exists.
      if (job.kind === "url") {
        job.kind = "file";
        job.path = payload.output;
        void this.refresh(job.id);
      }
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

  /** The job whose editor is open. */
  get editing(): Job | null {
    return this.jobs.find((j) => j.id === this.editingId) ?? null;
  }

  async openEditor(id: string) {
    this.editingId = id;
    // The popup is too narrow to drag a crop rectangle in, so it grows.
    await invoke("set_editor_size", { expanded: true }).catch(() => {});
  }

  async closeEditor() {
    this.editingId = null;
    await invoke("set_editor_size", { expanded: false }).catch(() => {});
  }

  setEdits(id: string, edits: Edits) {
    const job = this.#find(id);
    if (!job) return;
    job.edits = edits;
    void this.#loadPlan(job);
  }

  /** What a job will actually be encoded with — its own answer, or the global one. */
  settingsFor(job: Job): Settings {
    return job.settings ?? this.settings;
  }

  /** Gives a job its own compression settings, from the editor. */
  updateJobSettings(id: string, patch: Partial<Settings>) {
    const job = this.#find(id);
    if (!job) return;

    const next = { ...this.settingsFor(job), ...patch };

    if (patch.videoCodec) {
      const allowed = containersFor(patch.videoCodec);
      if (!allowed.includes(next.container)) next.container = allowed[0];
    }
    const allowedAudio = audioCodecsFor(next.container);
    if (!allowedAudio.includes(next.audioCodec)) next.audioCodec = allowedAudio[0];

    if (patch.videoCodec !== undefined || patch.hardware !== undefined) {
      const valid = presetsFor(next.videoCodec, next.hardware).map((p) => p.value);
      if (!valid.includes(next.preset)) {
        next.preset = defaultPresetFor(next.videoCodec, next.hardware);
      }
    }

    job.settings = next;
    void this.#loadPlan(job);
  }

  /** Hands a job back to the global settings. */
  clearJobSettings(id: string) {
    const job = this.#find(id);
    if (!job) return;
    job.settings = null;
    void this.#loadPlan(job);
  }

  /** Shows a short-lived message and clears it again. */
  #say(message: string) {
    this.notice = message;
    setTimeout(() => {
      if (this.notice === message) this.notice = null;
    }, 5000);
  }

  #blankJob(id: string, kind: "file" | "url"): Job {
    return {
      id,
      kind,
      url: null,
      path: "",
      name: "",
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
      edits: emptyEdits(),
      passedThrough: false,
      knownDuration: null,
      settings: null,
    };
  }

  /**
   * Long videos wait to be started by hand. Squeezing a two-hour stream into
   * ten megabytes takes ages and produces something nobody wants — the useful
   * thing is almost always a section of it.
   *
   * This only applies when aiming at a size. "No limit" is exactly the right
   * answer for a long recording, so choosing it removes the objection.
   */
  #shouldHoldBack(job: Job): boolean {
    if (this.settingsFor(job).mode !== "size") return false;
    return (job.knownDuration ?? 0) > LONG_VIDEO_SECONDS;
  }

  /** Queues a pasted link. The fetch itself happens on a worker thread. */
  async addUrl(url: string) {
    const trimmed = url.trim();
    if (this.jobs.some((j) => j.url === trimmed && j.status !== "done")) return;

    const id = `job-${nextId++}`;
    this.jobs.push({
      ...this.#blankJob(id, "url"),
      url: trimmed,
      name: trimmed,
      stage: "Reading link…",
    });

    const job = this.#find(id);
    if (!job) return;

    try {
      const info = await invoke<UrlInfo>("inspect_url", { url: trimmed });
      job.name = info.title;
      job.knownDuration = info.duration;
      job.stage = "Ready";

      // Long recordings download as normal — `start` sees the duration and
      // stops before compressing, leaving a file ready to trim.
      if (this.autoStart) await this.start(id);
    } catch (err) {
      job.status = "failed";
      job.stage = "Failed";
      job.error = String(err);
    }
  }

  /** Adds dropped or picked files, probing each before it appears settled. */
  async addFiles(paths: string[]) {
    // Refuse anything that clearly isn't video up front. Letting it through
    // would just make a job that fails a second later at the probe.
    const videos = paths.filter(isVideoFile);
    const rejected = paths.length - videos.length;
    if (rejected > 0) {
      this.#say(
        videos.length === 0
          ? rejected === 1
            ? "That isn't a video file."
            : "Those aren't video files."
          : `Skipped ${rejected} file${rejected === 1 ? "" : "s"} that ${rejected === 1 ? "isn't" : "aren't"} video.`,
      );
    }

    const fresh = videos.filter(
      (p) => !this.jobs.some((j) => j.path === p && j.status !== "done"),
    );

    for (const path of fresh) {
      const id = `job-${nextId++}`;
      this.jobs.push({
        ...this.#blankJob(id, "file"),
        path,
        name: path.split(/[\\/]/).pop() ?? path,
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
        job.knownDuration = details.info.duration;
        await this.#loadPlan(job);

        // Nothing to fetch for a local file, so a long one simply waits.
        if (this.#shouldHoldBack(job)) {
          job.status = "held";
          job.stage = "Waiting for you";
          continue;
        }

        job.stage = "Ready";
        if (this.autoStart) await this.start(job.id);
      } catch (err) {
        job.status = "failed";
        job.stage = "Failed";
        job.error = String(err);
      }
    }
  }

  async #loadPlan(job: Job) {
    if (!job.path) return;
    try {
      job.plan = await invoke<Plan>("preview_plan", {
        path: job.path,
        settings: $state.snapshot(this.settingsFor(job)),
        edits: $state.snapshot(job.edits),
      });
    } catch {
      job.plan = null;
    }
  }

  /** Re-reads a job's file, for when it has changed underneath us. */
  async refresh(id: string) {
    const job = this.#find(id);
    if (!job?.path) return;
    try {
      const details = await invoke<FileInfo>("inspect_file", { path: job.path });
      job.info = details.info;
      job.thumbnail = details.thumbnail;
      await this.#loadPlan(job);
    } catch {
      // The file may have been moved or deleted; leave what we have.
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

    // A long recording still gets downloaded — you need the file in front of
    // you to choose a section — it just stops before compressing.
    const holdBack = job.kind === "url" && this.#shouldHoldBack(job);
    const settings = {
      ...$state.snapshot(this.settingsFor(job)),
      ...(holdBack ? { autoCompressDownloads: false } : {}),
    };

    try {
      await invoke("start_job", {
        id: job.id,
        // A link that hasn't been fetched yet has no path to send.
        path: job.kind === "file" ? job.path : null,
        url: job.kind === "url" ? job.url : null,
        title: job.name,
        settings,
        edits: $state.snapshot(job.edits),
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
      (j) =>
        j.status === "running" || j.status === "queued" || j.status === "held",
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
