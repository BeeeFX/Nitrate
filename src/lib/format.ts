/** Decimal megabytes, matching how upload limits are advertised. */
export function formatSize(bytes: number): string {
  const KB = 1000;
  if (bytes >= KB ** 3) return `${(bytes / KB ** 3).toFixed(2)} GB`;
  if (bytes >= KB ** 2) return `${(bytes / KB ** 2).toFixed(1)} MB`;
  if (bytes >= KB) return `${Math.round(bytes / KB)} KB`;
  return `${bytes} B`;
}

export function formatDuration(seconds: number): string {
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return `${m}:${String(s).padStart(2, "0")}`;
}

export function formatBitrate(kbps: number): string {
  return kbps >= 1000 ? `${(kbps / 1000).toFixed(1)} Mbps` : `${Math.round(kbps)} kbps`;
}

export function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "";
  if (seconds < 60) return `${Math.ceil(seconds)}s left`;
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  if (m < 60) return `${m}m ${s}s left`;
  return `${Math.floor(m / 60)}h ${m % 60}m left`;
}

/** Middle-truncates long filenames so both the name and extension stay readable. */
export function truncateName(name: string, max = 34): string {
  if (name.length <= max) return name;
  const keep = Math.floor((max - 1) / 2);
  return `${name.slice(0, keep)}…${name.slice(-keep)}`;
}

/**
 * Reads a time the way people write one: `1:02:03`, `62:03`, `83`, `83s` or
 * `1h2m3s`. Null for anything else.
 */
export function parseClock(text: string): number | null {
  const t = text.trim().toLowerCase();
  if (!t) return null;

  const units = /^(?:(\d+(?:\.\d+)?)h)?(?:(\d+(?:\.\d+)?)m)?(?:(\d+(?:\.\d+)?)s?)?$/.exec(t);
  if (units && units.slice(1).some((part) => part !== undefined)) {
    const [h, m, s] = units.slice(1).map((part) => Number(part ?? 0));
    return h * 3600 + m * 60 + s;
  }

  const parts = t.split(":");
  if (parts.length > 3 || parts.some((p) => !/^\d+(\.\d+)?$/.test(p))) return null;
  return parts.reduce((total, part) => total * 60 + Number(part), 0);
}

/**
 * The moment a link was shared at, if it says: YouTube's `t=` and `start=`,
 * Twitch's `t=1h2m3s`, or the same in the fragment.
 */
export function linkTimestamp(url: string): number | null {
  try {
    const parsed = new URL(url);
    const fragment = new URLSearchParams(parsed.hash.replace(/^#/, ""));
    for (const key of ["t", "start", "time_continue"]) {
      const raw = parsed.searchParams.get(key) ?? fragment.get(key);
      const seconds = raw === null ? null : parseClock(raw);
      if (seconds !== null && seconds > 0) return seconds;
    }
  } catch {
    // Not a URL we can read; no timestamp, then.
  }
  return null;
}
