export interface MediaInfo {
  duration: number;
  width: number;
  height: number;
  fps: number;
  videoCodec: string;
  audioCodec: string | null;
  audioBitrateKbps: number | null;
  sizeBytes: number;
}

export interface FileInfo {
  path: string;
  name: string;
  info: MediaInfo;
  thumbnail: string | null;
}

export interface Plan {
  mode: TargetMode;
  crf: number | null;
  videoKbps: number;
  audioKbps: number;
  width: number;
  height: number;
  fps: number;
  encoder: string;
  downscaled: boolean;
  /** Quality mode only — roughly what it will come out at. */
  estimatedBytes: number | null;
  notes: string[];
}

export type VideoCodec = "h264" | "h265" | "vp9" | "av1";
export type Container = "mp4" | "webm" | "mkv";
export type AudioCodec = "aac" | "opus" | "copy" | "none";

export type TargetMode = "size" | "quality";
export type QualityLevel = "small" | "balanced" | "high";

export interface Settings {
  mode: TargetMode;
  quality: QualityLevel;
  targetBytes: number;
  videoCodec: VideoCodec;
  container: Container;
  audioCodec: AudioCodec;
  audioBitrateKbps: number;
  hardware: boolean;
  maxHeight: number | null;
  maxFps: number | null;
  safetyMargin: number;
  preset: string;
  twoPass: boolean;
  outputDir: string | null;
  autoCompressDownloads: boolean;
  maxDownloadHeight: number;
}

/** Fractions of the source frame, so the editor needn't know the real size. */
export interface CropRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Edits {
  start: number | null;
  end: number | null;
  crop: CropRect | null;
}

export interface UrlInfo {
  title: string;
  duration: number | null;
  site: string;
  webpageUrl: string;
}

export function emptyEdits(): Edits {
  return { start: null, end: null, crop: null };
}

export function hasEdits(edits: Edits): boolean {
  return edits.start !== null || edits.end !== null || edits.crop !== null;
}

export type JobStatus =
  | "queued"
  | "running"
  | "done"
  | "failed"
  | "cancelled"
  /** Deliberately not started — too long to squeeze into a fixed size. */
  | "held";

export interface Job {
  id: string;
  /** A link has no local file until it's been fetched. */
  kind: "file" | "url";
  url: string | null;
  /** Empty until a link has been resolved or a file probed. */
  path: string;
  name: string;
  info: MediaInfo | null;
  thumbnail: string | null;
  status: JobStatus;
  progress: number;
  stage: string;
  plan: Plan | null;
  output: string | null;
  finalBytes: number | null;
  originalBytes: number | null;
  error: string | null;
  notes: string[];
  startedAt: number | null;
  edits: Edits;
  /** Finished without re-encoding, because it already fitted. */
  passedThrough: boolean;
  /**
   * Length in seconds, known before there's a local file — a link reports its
   * duration up front, which is how a long VOD can be spotted without
   * downloading gigabytes first.
   */
  knownDuration: number | null;
  /**
   * Why a held job is waiting. The two reasons need different explanations —
   * one is about length, the other about where the link came from.
   */
  heldReason: "long" | "browser" | null;
  /**
   * Per-video overrides set in the editor. Null means "whatever the main
   * interface says", so changing the target there still moves everything that
   * hasn't been given its own answer.
   */
  settings: Settings | null;
}

export interface Capabilities {
  hardwareEncoders: string[];
  ffmpegFound: boolean;
}
