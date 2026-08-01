//! Turning "make this fit in N bytes" into an ffmpeg command line.
//!
//! The core idea is simple arithmetic — a file's size is just its bitrate times
//! its duration — but hitting a hard ceiling reliably needs three extra pieces:
//! a safety margin for container overhead, a quality floor that downscales rather
//! than producing an unwatchable smear, and a verify-and-retry loop because
//! rate control is a target, not a guarantee.

use crate::ffmpeg::{self, Binaries, Cancelled, MediaInfo, NULL_SINK};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    H265,
    Vp9,
    Av1,
}

impl VideoCodec {
    fn software(self) -> &'static str {
        match self {
            VideoCodec::H264 => "libx264",
            VideoCodec::H265 => "libx265",
            VideoCodec::Vp9 => "libvpx-vp9",
            VideoCodec::Av1 => "libsvtav1",
        }
    }

    /// Hardware encoders in rough order of preference, per vendor.
    fn hardware(self) -> &'static [&'static str] {
        match self {
            VideoCodec::H264 => &["h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox"],
            VideoCodec::H265 => &["hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_videotoolbox"],
            VideoCodec::Av1 => &["av1_nvenc", "av1_qsv"],
            VideoCodec::Vp9 => &["vp9_qsv"],
        }
    }

    /// Bits per pixel per frame below which this codec stops looking acceptable.
    /// Newer codecs hold up at lower rates, so they tolerate a smaller floor.
    fn min_bpp(self) -> f64 {
        match self {
            VideoCodec::H264 => 0.045,
            VideoCodec::H265 => 0.030,
            VideoCodec::Vp9 => 0.032,
            VideoCodec::Av1 => 0.025,
        }
    }

    fn supports_two_pass(self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Mp4,
    Webm,
    Mkv,
}

impl Container {
    fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Webm => "webm",
            Container::Mkv => "mkv",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    Aac,
    Opus,
    Copy,
    None,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub target_bytes: u64,
    pub video_codec: VideoCodec,
    pub container: Container,
    pub audio_codec: AudioCodec,
    pub audio_bitrate_kbps: u32,
    pub hardware: bool,
    /// `None` lets the planner pick a resolution that fits the bitrate.
    pub max_height: Option<u32>,
    pub max_fps: Option<f64>,
    /// Fraction of the target we actually aim for, leaving room for muxing slop.
    pub safety_margin: f64,
    pub preset: String,
    pub two_pass: bool,
    pub output_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_bytes: 10 * 1000 * 1000,
            video_codec: VideoCodec::H264,
            container: Container::Mp4,
            audio_codec: AudioCodec::Aac,
            audio_bitrate_kbps: 128,
            hardware: false,
            max_height: None,
            max_fps: None,
            safety_margin: 0.97,
            preset: "medium".into(),
            two_pass: true,
            output_dir: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub video_kbps: u32,
    pub audio_kbps: u32,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub encoder: String,
    pub downscaled: bool,
    pub notes: Vec<String>,
}

/// Standard heights we're willing to step down through.
const LADDER: &[u32] = &[2160, 1440, 1080, 900, 720, 540, 480, 360, 270, 180];

/// Below this, video stops being worth encoding at all.
const MIN_VIDEO_KBPS: f64 = 48.0;

pub fn plan(info: &MediaInfo, settings: &Settings, bins: &Binaries) -> Result<Plan, String> {
    let mut notes = Vec::new();

    let margin = settings.safety_margin.clamp(0.5, 0.999);
    let usable_bits = settings.target_bytes as f64 * 8.0 * margin;
    let total_kbps = usable_bits / 1000.0 / info.duration;

    // Audio comes off the top — it's effectively fixed cost.
    let mut audio_kbps = match settings.audio_codec {
        AudioCodec::None => 0,
        AudioCodec::Copy => info
            .audio_bitrate_kbps
            .unwrap_or(settings.audio_bitrate_kbps),
        _ => settings.audio_bitrate_kbps,
    };
    if !info.has_audio() {
        audio_kbps = 0;
    }

    // On very tight targets, a fixed 128k audio track can eat the whole budget.
    // Shrinking audio is far less visible than starving the video.
    let audio_floor = match settings.audio_codec {
        AudioCodec::Opus => 32,
        _ => 48,
    };
    if audio_kbps > 0 && (total_kbps - audio_kbps as f64) < MIN_VIDEO_KBPS * 2.0 {
        let squeezed = ((total_kbps * 0.25) as u32).clamp(audio_floor, audio_kbps);
        if squeezed < audio_kbps && settings.audio_codec != AudioCodec::Copy {
            notes.push(format!(
                "Audio dropped to {squeezed} kbps to protect the video."
            ));
            audio_kbps = squeezed;
        }
    }

    // A little headroom for container overhead on top of the safety margin.
    let video_kbps = total_kbps - audio_kbps as f64 - (total_kbps * 0.01);

    if video_kbps < MIN_VIDEO_KBPS {
        return Err(format!(
            "{} is too small for a {} video — even at minimum quality it won't fit. Try a larger target or trim the clip.",
            crate::format_size(settings.target_bytes),
            format_duration(info.duration)
        ));
    }

    // Work out the frame rate first, since it feeds the bits-per-pixel maths.
    let mut fps = info.fps;
    if let Some(cap) = settings.max_fps {
        if fps > cap {
            fps = cap;
            notes.push(format!("Frame rate capped at {} fps.", cap.round()));
        }
    }

    // Then pick the largest resolution the bitrate can actually carry.
    let source_height = info.height.max(1);
    let mut height = settings
        .max_height
        .unwrap_or(source_height)
        .min(source_height);
    let min_bpp = settings.video_codec.min_bpp();

    let bpp_at = |h: u32, fps: f64| -> f64 {
        let w = scaled_width(info, h) as f64;
        (video_kbps * 1000.0) / (w * h as f64 * fps)
    };

    if settings.max_height.is_none() {
        // Auto mode: step down the ladder until the picture has enough bits.
        while bpp_at(height, fps) < min_bpp {
            // Halving the frame rate is usually less damaging than another
            // resolution step, so try that once before dropping further.
            if fps > 30.0 && settings.max_fps.is_none() {
                fps = 30.0;
                notes.push("Frame rate reduced to 30 fps to fit the target.".into());
                continue;
            }
            match LADDER.iter().copied().find(|&h| h < height) {
                Some(next) => height = next,
                None => break,
            }
        }
        if height < source_height {
            notes.push(format!(
                "Downscaled to {height}p to keep the picture clean."
            ));
        }
    }

    let width = scaled_width(info, height);

    // Pick the actual encoder, falling back to software if the GPU can't help.
    let mut encoder = settings.video_codec.software().to_string();
    if settings.hardware {
        let available = ffmpeg::available_encoders(bins);
        let pick = settings
            .video_codec
            .hardware()
            .iter()
            .find(|candidate| {
                available.iter().any(|e| e == *candidate) && ffmpeg::encoder_works(bins, candidate)
            })
            .copied();

        match pick {
            Some(hw) => {
                encoder = hw.to_string();
                notes.push(format!(
                    "Using {hw} — fast, but size accuracy is looser than software."
                ));
            }
            None => notes.push("No usable hardware encoder found; using software.".into()),
        }
    }

    Ok(Plan {
        video_kbps: video_kbps.max(MIN_VIDEO_KBPS).round() as u32,
        audio_kbps,
        width,
        height,
        fps,
        encoder,
        downscaled: height < source_height,
        notes,
    })
}

/// Keeps the aspect ratio and forces an even width, which every codec requires.
fn scaled_width(info: &MediaInfo, height: u32) -> u32 {
    if info.height == 0 || info.width == 0 {
        return 0;
    }
    let ratio = info.width as f64 / info.height as f64;
    let w = (height as f64 * ratio).round() as u32;
    w.max(2) & !1
}

fn format_duration(secs: f64) -> String {
    let total = secs.round() as u64;
    let (m, s) = (total / 60, total % 60);
    if m >= 60 {
        format!("{}h{:02}m", m / 60, m % 60)
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

// ---------------------------------------------------------------------------
// Command construction
// ---------------------------------------------------------------------------

fn s(v: &str) -> String {
    v.to_string()
}

/// Builds the argument list for one ffmpeg pass.
///
/// `pass` is `None` for a single-pass encode, or `Some(1 | 2)` for two-pass.
fn build_args(
    input: &Path,
    output: &Path,
    plan: &Plan,
    settings: &Settings,
    info: &MediaInfo,
    pass: Option<u8>,
    passlog: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = vec![s("-y"), s("-i"), input.to_string_lossy().into_owned()];

    // Only build a filter chain if something actually changes.
    let mut filters: Vec<String> = Vec::new();
    if plan.height != info.height {
        filters.push(format!("scale=-2:{}:flags=lanczos", plan.height));
    }
    if (plan.fps - info.fps).abs() > 0.01 {
        filters.push(format!("fps={:.3}", plan.fps));
    }
    if !filters.is_empty() {
        args.push(s("-vf"));
        args.push(filters.join(","));
    }

    args.push(s("-c:v"));
    args.push(plan.encoder.clone());
    args.push(s("-b:v"));
    args.push(format!("{}k", plan.video_kbps));

    // Encoder-specific knobs.
    let is_hardware = !plan.encoder.starts_with("lib");
    match plan.encoder.as_str() {
        "libx264" | "libx265" => {
            args.push(s("-preset"));
            args.push(settings.preset.clone());
        }
        "libvpx-vp9" => {
            // Multithreaded row encoding; VP9 is painfully slow without it.
            args.extend([
                s("-row-mt"),
                s("1"),
                s("-deadline"),
                s("good"),
                s("-cpu-used"),
                s("2"),
            ]);
        }
        "libsvtav1" => {
            args.push(s("-preset"));
            args.push(s("6"));
        }
        _ => {}
    }

    if is_hardware {
        // Hardware rate control drifts, so clamp it with an explicit ceiling.
        args.extend([
            s("-maxrate"),
            format!("{}k", (plan.video_kbps as f64 * 1.35) as u32),
            s("-bufsize"),
            format!("{}k", plan.video_kbps * 2),
        ]);
    }

    // H.265 in MP4 needs the hvc1 tag or most players refuse it.
    if settings.container == Container::Mp4 && plan.encoder.contains("265")
        || settings.container == Container::Mp4 && plan.encoder.contains("hevc")
    {
        args.extend([s("-tag:v"), s("hvc1")]);
    }

    // Broad-compatibility pixel format — some sources are 10-bit or 4:2:2,
    // which many players (and Discord's inline preview) won't touch.
    if !is_hardware && matches!(settings.video_codec, VideoCodec::H264 | VideoCodec::H265) {
        args.extend([s("-pix_fmt"), s("yuv420p")]);
    }

    if let Some(n) = pass {
        args.extend([
            s("-pass"),
            n.to_string(),
            s("-passlogfile"),
            passlog.to_string_lossy().into_owned(),
        ]);
    }

    if pass == Some(1) {
        // Analysis pass: no audio, no output file.
        args.extend([s("-an"), s("-f"), s("null"), s(NULL_SINK)]);
        return args;
    }

    // Audio, on the real pass only.
    if !info.has_audio() || settings.audio_codec == AudioCodec::None {
        args.push(s("-an"));
    } else {
        match settings.audio_codec {
            AudioCodec::Copy => args.extend([s("-c:a"), s("copy")]),
            AudioCodec::Opus => args.extend([
                s("-c:a"),
                s("libopus"),
                s("-b:a"),
                format!("{}k", plan.audio_kbps),
            ]),
            _ => args.extend([
                s("-c:a"),
                s("aac"),
                s("-b:a"),
                format!("{}k", plan.audio_kbps),
                // Downmix to stereo; surround sources waste bitrate Discord won't play back.
                s("-ac"),
                s("2"),
            ]),
        }
    }

    if settings.container == Container::Mp4 {
        // Puts the index at the front so the file starts playing before it's
        // fully downloaded — which is exactly how Discord serves it.
        args.extend([s("-movflags"), s("+faststart")]);
    }

    args.push(output.to_string_lossy().into_owned());
    args
}

// ---------------------------------------------------------------------------
// Running the job
// ---------------------------------------------------------------------------

pub struct EncodeOutcome {
    pub output: PathBuf,
    pub final_bytes: u64,
    pub attempts: u32,
    pub plan: Plan,
}

/// How close to the target we consider "done" — overshooting is fatal, but
/// landing far under wastes quality, so a retry only triggers on overshoot.
const MAX_ATTEMPTS: u32 = 3;

/// Distinguishes concurrent jobs' two-pass log files.
static PASSLOG_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn run_job(
    bins: &Binaries,
    input: &Path,
    settings: &Settings,
    info: &MediaInfo,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(f64, &str),
) -> Result<Result<EncodeOutcome, Cancelled>, String> {
    let mut plan = plan(info, settings, bins)?;
    let output = output_path(input, settings)?;

    // Pass logs are noisy temp files; keep them out of the user's folders.
    //
    // The name must be unique per *job*, not per file: several jobs encode
    // concurrently, and two inputs from different folders can easily share a
    // stem. Colliding here silently corrupts pass-1 statistics and makes the
    // second pass fail with a bare "Invalid argument".
    let passlog = std::env::temp_dir().join(format!(
        "nitrate-{}-{}",
        std::process::id(),
        PASSLOG_SEQ.fetch_add(1, Ordering::Relaxed)
    ));

    let two_pass = settings.two_pass
        && settings.video_codec.supports_two_pass()
        && plan.encoder.starts_with("lib");

    let mut attempts = 0;
    let mut final_bytes;

    loop {
        attempts += 1;

        if two_pass {
            let args = build_args(input, &output, &plan, settings, info, Some(1), &passlog);
            let stage = if attempts > 1 {
                "Analysing (retry)"
            } else {
                "Analysing"
            };
            match ffmpeg::run(bins, &args, info.duration, cancel, |p| {
                on_progress(p * 0.35, stage)
            })? {
                Ok(()) => {}
                Err(c) => {
                    cleanup_passlog(&passlog);
                    return Ok(Err(c));
                }
            }

            let args = build_args(input, &output, &plan, settings, info, Some(2), &passlog);
            match ffmpeg::run(bins, &args, info.duration, cancel, |p| {
                on_progress(0.35 + p * 0.65, "Encoding")
            })? {
                Ok(()) => {}
                Err(c) => {
                    cleanup_passlog(&passlog);
                    return Ok(Err(c));
                }
            }
        } else {
            let args = build_args(input, &output, &plan, settings, info, None, &passlog);
            match ffmpeg::run(bins, &args, info.duration, cancel, |p| {
                on_progress(p, "Encoding")
            })? {
                Ok(()) => {}
                Err(c) => {
                    cleanup_passlog(&passlog);
                    return Ok(Err(c));
                }
            }
        }

        final_bytes = std::fs::metadata(&output)
            .map_err(|e| format!("Couldn't read the finished file: {e}"))?
            .len();

        // Rate control aims, it doesn't promise. If we overshot, scale the
        // bitrate by how far off we were and go again.
        if final_bytes <= settings.target_bytes || attempts >= MAX_ATTEMPTS {
            break;
        }

        let correction = (settings.target_bytes as f64 / final_bytes as f64) * 0.97;
        let corrected = (plan.video_kbps as f64 * correction).max(MIN_VIDEO_KBPS);
        plan.video_kbps = corrected.round() as u32;
        plan.notes
            .push(format!("Overshot; retrying at {} kbps.", plan.video_kbps));
        on_progress(0.0, "Retrying");
    }

    cleanup_passlog(&passlog);

    if final_bytes > settings.target_bytes {
        return Err(format!(
            "Couldn't get under {} after {attempts} attempts (landed at {}). Try a lower resolution or a newer codec.",
            crate::format_size(settings.target_bytes),
            crate::format_size(final_bytes)
        ));
    }

    Ok(Ok(EncodeOutcome {
        output,
        final_bytes,
        attempts,
        plan,
    }))
}

/// Picks a destination that won't clobber anything the user already has.
fn output_path(input: &Path, settings: &Settings) -> Result<PathBuf, String> {
    let dir = match &settings.output_dir {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => dirs_downloads().ok_or("Couldn't find your Downloads folder.")?,
    };

    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create the output folder: {e}"))?;

    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "video".into());
    let ext = settings.container.extension();

    let mut candidate = dir.join(format!("{stem}-nitrate.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}-nitrate-{n}.{ext}"));
        n += 1;
        if n > 999 {
            return Err("Too many files with that name already.".into());
        }
    }
    Ok(candidate)
}

fn dirs_downloads() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|p| p.join("Downloads"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join("Downloads"))
    }
}

/// ffmpeg leaves `<log>-0.log` and `<log>-0.log.mbtree` behind.
fn cleanup_passlog(passlog: &Path) {
    let Some(dir) = passlog.parent() else { return };
    let Some(prefix) = passlog
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
    else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
