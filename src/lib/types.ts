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
  videoKbps: number;
  audioKbps: number;
  width: number;
  height: number;
  fps: number;
  encoder: string;
  downscaled: boolean;
  notes: string[];
}

export type VideoCodec = "h264" | "h265" | "vp9" | "av1";
export type Container = "mp4" | "webm" | "mkv";
export type AudioCodec = "aac" | "opus" | "copy" | "none";

export interface Settings {
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
}

export type JobStatus =
  | "queued"
  | "running"
  | "done"
  | "failed"
  | "cancelled";

export interface Job {
  id: string;
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
}

export interface Capabilities {
  hardwareEncoders: string[];
  ffmpegFound: boolean;
}
