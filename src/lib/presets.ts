import type { AudioCodec, Container, Settings, VideoCodec } from "./types";

export interface Tier {
  id: string;
  label: string;
  sub: string;
  bytes: number;
}

/**
 * Discord's upload ceilings. Sizes are decimal MB, which combined with the
 * safety margin keeps us clear of the limit however Discord counts it.
 */
export const TIERS: Tier[] = [
  { id: "free", label: "Free", sub: "10 MB", bytes: 10_000_000 },
  { id: "basic", label: "Nitro Basic", sub: "50 MB", bytes: 50_000_000 },
  { id: "boost", label: "Boost L3", sub: "100 MB", bytes: 100_000_000 },
  { id: "nitro", label: "Nitro", sub: "500 MB", bytes: 500_000_000 },
];

export const VIDEO_CODECS: {
  id: VideoCodec;
  label: string;
  hint: string;
}[] = [
  { id: "h264", label: "H.264", hint: "Plays everywhere. Best default." },
  { id: "h265", label: "H.265", hint: "~30% smaller, spotty inline preview." },
  { id: "vp9", label: "VP9", hint: "Good quality, slow to encode." },
  { id: "av1", label: "AV1", hint: "Smallest files, newest support." },
];

export const CONTAINERS: { id: Container; label: string }[] = [
  { id: "mp4", label: "MP4" },
  { id: "webm", label: "WebM" },
  { id: "mkv", label: "MKV" },
];

export const AUDIO_CODECS: { id: AudioCodec; label: string }[] = [
  { id: "aac", label: "AAC" },
  { id: "opus", label: "Opus" },
  { id: "copy", label: "Copy" },
  { id: "none", label: "None" },
];

export const PRESETS = [
  "ultrafast",
  "superfast",
  "veryfast",
  "faster",
  "fast",
  "medium",
  "slow",
  "slower",
];

export const AUDIO_BITRATES = [64, 96, 128, 160, 192, 256];

/** Codec/container pairs that actually mux. */
export function containersFor(codec: VideoCodec): Container[] {
  switch (codec) {
    case "vp9":
      return ["webm", "mkv"];
    case "av1":
      return ["mp4", "webm", "mkv"];
    default:
      return ["mp4", "mkv"];
  }
}

export function audioCodecsFor(container: Container): AudioCodec[] {
  return container === "webm"
    ? ["opus", "copy", "none"]
    : ["aac", "opus", "copy", "none"];
}

export const DEFAULT_SETTINGS: Settings = {
  targetBytes: 10_000_000,
  videoCodec: "h264",
  container: "mp4",
  audioCodec: "aac",
  audioBitrateKbps: 128,
  hardware: false,
  maxHeight: null,
  maxFps: null,
  safetyMargin: 0.97,
  preset: "medium",
  twoPass: true,
  outputDir: null,
};

export const RESOLUTION_CAPS = [
  { value: null, label: "Auto" },
  { value: 1080, label: "1080p" },
  { value: 720, label: "720p" },
  { value: 480, label: "480p" },
  { value: 360, label: "360p" },
];

export const FPS_CAPS = [
  { value: null, label: "Source" },
  { value: 60, label: "60" },
  { value: 30, label: "30" },
  { value: 24, label: "24" },
];
