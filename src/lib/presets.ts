import type {
  AudioCodec,
  Container,
  QualityLevel,
  Settings,
  VideoCodec,
} from "./types";

/**
 * What the drop zone and file picker will take.
 *
 * Dropping a PDF used to create a job that failed a second later at the probe;
 * refusing it up front is both faster and kinder.
 */
export const VIDEO_EXTENSIONS = [
  "mp4", "mov", "mkv", "webm", "avi", "wmv", "flv", "m4v",
  "mpg", "mpeg", "ts", "m2ts", "mts", "3gp", "ogv", "vob",
  "asf", "rm", "rmvb", "divx", "f4v", "m2v", "mxf", "gif",
];

export function isVideoFile(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return VIDEO_EXTENSIONS.includes(ext);
}

/** Quality mode: squeeze it sensibly and let the size land where it lands. */
export const QUALITY_LEVELS: { id: QualityLevel; label: string; hint: string }[] = [
  { id: "small", label: "Smallest", hint: "Hardest squeeze. Visible on detailed footage." },
  { id: "balanced", label: "Balanced", hint: "Good quality at a fraction of the size." },
  { id: "high", label: "Best", hint: "Near-original. Still much smaller than the source." },
];

export interface Tier {
  id: string;
  label: string;
  sub: string;
  bytes: number;
}

/**
 * Discord's upload ceilings. Sizes are decimal MB, which combined with the
 * safety margin keeps us clear of the limit however Discord counts it.
 *
 * The free ceiling moved from 10 MB to 20 MB, and the old one is kept beside
 * it: the raise reached clients gradually, and a server that hasn't caught up
 * still bounces anything over ten. Better to offer both than to have someone
 * compress twice.
 */
export const TIERS: Tier[] = [
  { id: "free-old", label: "Old Free", sub: "10 MB", bytes: 10_000_000 },
  { id: "free", label: "New Free", sub: "20 MB", bytes: 20_000_000 },
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

export interface PresetOption {
  value: string;
  label: string;
}

/**
 * Every encoder spells "how hard should I work" differently: x264 and x265 take
 * named presets, VP9 takes a `cpu-used` number that counts the wrong way round,
 * SVT-AV1 takes 0–13, and each GPU vendor has its own scale again. Rather than
 * leak that, hardware encoders get three plain choices which `encode.rs` maps
 * to whatever the chosen encoder actually wants.
 */
const X264_PRESETS: PresetOption[] = [
  { value: "ultrafast", label: "Ultrafast" },
  { value: "superfast", label: "Superfast" },
  { value: "veryfast", label: "Very fast" },
  { value: "faster", label: "Faster" },
  { value: "fast", label: "Fast" },
  { value: "medium", label: "Medium" },
  { value: "slow", label: "Slow" },
  { value: "slower", label: "Slower (best)" },
];

// VP9's scale is inverted: higher cpu-used means less effort.
const VP9_PRESETS: PresetOption[] = [
  { value: "5", label: "Fastest" },
  { value: "4", label: "Faster" },
  { value: "3", label: "Fast" },
  { value: "2", label: "Balanced" },
  { value: "1", label: "Slow" },
  { value: "0", label: "Slowest (best)" },
];

const AV1_PRESETS: PresetOption[] = [
  { value: "10", label: "Fastest" },
  { value: "8", label: "Faster" },
  { value: "6", label: "Balanced" },
  { value: "4", label: "Slow" },
  { value: "2", label: "Slower (best)" },
];

const HARDWARE_PRESETS: PresetOption[] = [
  { value: "speed", label: "Fastest" },
  { value: "balanced", label: "Balanced" },
  { value: "quality", label: "Best quality" },
];

export function presetsFor(codec: VideoCodec, hardware: boolean): PresetOption[] {
  if (hardware) return HARDWARE_PRESETS;
  switch (codec) {
    case "vp9":
      return VP9_PRESETS;
    case "av1":
      return AV1_PRESETS;
    default:
      return X264_PRESETS;
  }
}

export function defaultPresetFor(codec: VideoCodec, hardware: boolean): string {
  if (hardware) return "balanced";
  switch (codec) {
    case "vp9":
      return "2";
    case "av1":
      return "6";
    default:
      return "medium";
  }
}

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
  mode: "size",
  quality: "balanced",
  targetBytes: 20_000_000,
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
  autoCompressDownloads: true,
  maxDownloadHeight: 1080,
};

/**
 * Past this, a video won't start compressing on its own.
 *
 * Squeezing an hours-long stream VOD into ten megabytes gives you something
 * unwatchable after a very long wait, and it's almost never what was wanted —
 * usually one moment out of it was. So it waits for you to trim it.
 */
export const LONG_VIDEO_SECONDS = 20 * 60;

export const DOWNLOAD_HEIGHTS = [
  { value: 2160, label: "4K" },
  { value: 1440, label: "1440p" },
  { value: 1080, label: "1080p" },
  { value: 720, label: "720p" },
];

/** Aspect presets for the crop tool, as width ÷ height. `null` is freeform. */
export const ASPECTS: { id: string; label: string; ratio: number | null }[] = [
  { id: "free", label: "Free", ratio: null },
  { id: "16:9", label: "16:9", ratio: 16 / 9 },
  { id: "1:1", label: "1:1", ratio: 1 },
  { id: "9:16", label: "9:16", ratio: 9 / 16 },
  { id: "4:5", label: "4:5", ratio: 4 / 5 },
];

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
