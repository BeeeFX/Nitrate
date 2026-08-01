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
