import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready"
  | "error";

/** Re-check occasionally so a long-running tray app doesn't go stale. */
const RECHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

class Updater {
  status = $state<UpdateStatus>("idle");
  version = $state<string | null>(null);
  notes = $state<string | null>(null);
  /** 0–1 while downloading; null when the server sends no content-length. */
  progress = $state<number | null>(null);
  error = $state<string | null>(null);
  dismissed = $state(false);

  #update: Update | null = null;
  #timer: ReturnType<typeof setInterval> | null = null;

  /** True when there's something worth showing the user. */
  get visible() {
    return (
      !this.dismissed &&
      (this.status === "available" ||
        this.status === "downloading" ||
        this.status === "ready")
    );
  }

  start() {
    void this.check();
    this.#timer ??= setInterval(() => void this.check(), RECHECK_INTERVAL_MS);
  }

  stop() {
    if (this.#timer) clearInterval(this.#timer);
    this.#timer = null;
  }

  async check() {
    // Nothing to do if an update is already in flight.
    if (this.status === "downloading" || this.status === "ready") return;

    this.status = "checking";
    this.error = null;

    try {
      const update = await check();

      if (!update) {
        this.status = "idle";
        return;
      }

      this.#update = update;
      this.version = update.version;
      this.notes = update.body ?? null;
      this.dismissed = false;
      this.status = "available";
    } catch (err) {
      // A dev build has no updater endpoint, and being offline is normal.
      // Neither is worth putting in the user's face.
      this.status = "idle";
      this.error = String(err);
    }
  }

  async install() {
    if (!this.#update) return;

    this.status = "downloading";
    this.progress = null;

    let total = 0;
    let received = 0;

    try {
      await this.#update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            this.progress = total > 0 ? 0 : null;
            break;
          case "Progress":
            received += event.data.chunkLength;
            if (total > 0) this.progress = Math.min(received / total, 1);
            break;
          case "Finished":
            this.progress = 1;
            break;
        }
      });

      this.status = "ready";
    } catch (err) {
      this.status = "error";
      this.error = String(err);
    }
  }

  async restart() {
    await relaunch();
  }

  dismiss() {
    this.dismissed = true;
  }
}

export const updater = new Updater();
