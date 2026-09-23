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

/// Two ways to ask for a smaller file.
///
/// `Size` aims at an exact ceiling, which is what Discord needs. `Quality`
/// just encodes well and accepts whatever size results — the sane choice for a
/// long recording, where picking a megabyte figure is guesswork.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetMode {
    Size,
    Quality,
    /// Apply the edits and change nothing else.
    ///
    /// A downloaded video has already been compressed once by whoever it came
    /// from, and re-encoding it to trim off the first ten seconds only throws
    /// away quality for nothing. Where the edits allow it the streams are
    /// copied verbatim, which is both lossless and near-instant.
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityLevel {
    Small,
    Balanced,
    High,
}

impl QualityLevel {
    /// Constant-quality values, which mean different things to each encoder.
    fn crf(self, codec: VideoCodec) -> u32 {
        match (codec, self) {
            (VideoCodec::H264, QualityLevel::Small) => 30,
            (VideoCodec::H264, QualityLevel::Balanced) => 24,
            (VideoCodec::H264, QualityLevel::High) => 20,

            (VideoCodec::H265, QualityLevel::Small) => 32,
            (VideoCodec::H265, QualityLevel::Balanced) => 27,
            (VideoCodec::H265, QualityLevel::High) => 23,

            (VideoCodec::Vp9, QualityLevel::Small) => 38,
            (VideoCodec::Vp9, QualityLevel::Balanced) => 32,
            (VideoCodec::Vp9, QualityLevel::High) => 28,

            (VideoCodec::Av1, QualityLevel::Small) => 45,
            (VideoCodec::Av1, QualityLevel::Balanced) => 35,
            (VideoCodec::Av1, QualityLevel::High) => 28,
        }
    }

    fn label(self) -> &'static str {
        match self {
            QualityLevel::Small => "smallest",
            QualityLevel::Balanced => "balanced",
            QualityLevel::High => "high quality",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub mode: TargetMode,
    pub quality: QualityLevel,
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
    /// When off, a pasted link is downloaded and then left alone, ready to be
    /// edited and compressed by hand.
    pub auto_compress_downloads: bool,
    /// Ceiling on what to fetch from a link. Pulling 4K only to squeeze it into
    /// ten megabytes wastes bandwidth and time for no gain.
    pub max_download_height: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: TargetMode::Size,
            quality: QualityLevel::Balanced,
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
            auto_compress_downloads: true,
            max_download_height: 1080,
        }
    }
}

// ---------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------

/// A crop, stored as fractions of the source rather than pixels so the editor
/// can work against a preview of any size without having to know the real
/// resolution.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edits {
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub crop: Option<CropRect>,
    /// The editor's compress button was pressed on this one.
    ///
    /// Not a picture edit, so it stays out of `is_empty` — it changes nothing
    /// about the ffmpeg command, only whether the file is allowed to skip it.
    /// A GIF is otherwise handed over untouched, and this is how someone says
    /// they meant it: shrinking one without cropping or trimming it is a
    /// perfectly reasonable thing to want, and there's no other way to ask.
    #[serde(default)]
    pub force_encode: bool,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.start.is_none() && self.end.is_none() && self.crop.is_none()
    }

    pub fn start_at(&self, full: f64) -> f64 {
        self.start.unwrap_or(0.0).clamp(0.0, full)
    }

    /// How much video actually gets encoded — and therefore what the bitrate
    /// budget is spread across. Trimming is the cheapest way to buy quality.
    pub fn duration(&self, full: f64) -> f64 {
        let start = self.start_at(full);
        let end = self.end.unwrap_or(full).clamp(start, full);
        (end - start).max(0.1)
    }

    /// Source dimensions after cropping, rounded to even numbers because
    /// every codec here demands it.
    pub fn dimensions(&self, width: u32, height: u32) -> (u32, u32) {
        match self.crop {
            Some(c) => (
                even(width as f64 * c.width.clamp(0.01, 1.0)),
                even(height as f64 * c.height.clamp(0.01, 1.0)),
            ),
            None => (width, height),
        }
    }

    /// The crop in pixels, ready for ffmpeg's filter.
    fn crop_filter(&self, width: u32, height: u32) -> Option<String> {
        let c = self.crop?;
        let (w, h) = self.dimensions(width, height);
        let x = even(width as f64 * c.x.clamp(0.0, 1.0)).min(width.saturating_sub(w));
        let y = even(height as f64 * c.y.clamp(0.0, 1.0)).min(height.saturating_sub(h));
        Some(format!("crop={w}:{h}:{x}:{y}"))
    }
}

fn even(value: f64) -> u32 {
    ((value.round() as u32).max(2)) & !1
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub mode: TargetMode,
    /// Zero in quality mode, where there's no bitrate budget to spend.
    pub video_kbps: u32,
    /// Set in quality mode only.
    pub crf: Option<u32>,
    pub audio_kbps: u32,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub encoder: String,
    pub downscaled: bool,
    /// Roughly what quality mode will produce. `None` when a size was asked
    /// for, since that answer is already the target.
    pub estimated_bytes: Option<u64>,
    /// Copy the streams instead of encoding them. Only ever set in keep mode,
    /// and only when the edits don't touch the picture.
    pub copy_streams: bool,
    pub notes: Vec<String>,
}

/// Bits per pixel at a reference CRF, per encoder family.
///
/// Constant-quality encoding has no size to report until it's done, so this is
/// the only way to answer "how big will it be" before running it. Each family
/// gets an anchor — a CRF and the bits per pixel it lands near on ordinary
/// footage — and the scale between anchors is the usual rule that six points of
/// CRF halves the bitrate.
///
/// It is genuinely an estimate. A static talking head and a snowstorm at the
/// same CRF differ by several times over, and nothing short of encoding can
/// tell them apart, which is why the number is shown with a "≈".
fn reference_bpp(codec: VideoCodec) -> (f64, f64) {
    match codec {
        VideoCodec::H264 => (23.0, 0.085),
        VideoCodec::H265 => (28.0, 0.060),
        VideoCodec::Vp9 => (32.0, 0.060),
        VideoCodec::Av1 => (35.0, 0.045),
    }
}

fn estimate_quality_bytes(
    codec: VideoCodec,
    crf: u32,
    width: u32,
    height: u32,
    fps: f64,
    audio_kbps: u32,
    duration: f64,
) -> Option<u64> {
    if duration <= 0.0 || width == 0 || height == 0 {
        return None;
    }

    let (reference_crf, reference) = reference_bpp(codec);
    let bpp = reference * 2f64.powf((reference_crf - crf as f64) / 6.0);

    let fps = if fps > 0.0 { fps } else { 30.0 };
    let video_bps = bpp * width as f64 * height as f64 * fps;
    let total_bps = video_bps + audio_kbps as f64 * 1000.0;

    Some((total_bps * duration / 8.0) as u64)
}

/// Standard heights we're willing to step down through.
const LADDER: &[u32] = &[2160, 1440, 1080, 900, 720, 540, 480, 360, 270, 180];

/// Below this, video stops being worth encoding at all.
const MIN_VIDEO_KBPS: f64 = 48.0;

/// Floors for shrinking a GIF towards a size target. Past these it stops being
/// worth having — a postage stamp at four frames a second is not the thing
/// anyone asked to keep as a GIF.
const MIN_GIF_HEIGHT: u32 = 120;
const MIN_GIF_FPS: f64 = 8.0;

pub fn plan(
    info: &MediaInfo,
    settings: &Settings,
    edits: &Edits,
    bins: &Binaries,
) -> Result<Plan, String> {
    let mut notes = Vec::new();

    // Everything downstream works from the *edited* clip: trimming shortens the
    // duration the budget is spread over, and cropping shrinks the frame that
    // has to be filled. Both buy quality, so they have to feed the maths rather
    // than be applied as an afterthought.
    let duration = edits.duration(info.duration);
    let (source_width, source_height) = edits.dimensions(info.width, info.height);

    // Quality mode short-circuits everything below: with no size to hit there's
    // no budget to divide, no bits-per-pixel floor to defend, and nothing to
    // verify afterwards. Just encode it well.
    if settings.mode == TargetMode::Quality {
        return plan_by_quality(
            info,
            settings,
            edits,
            bins,
            source_width,
            source_height,
            notes,
        );
    }

    if settings.mode == TargetMode::Keep {
        return plan_by_keeping(
            info,
            settings,
            edits,
            bins,
            source_width,
            source_height,
            notes,
        );
    }

    let margin = settings.safety_margin.clamp(0.5, 0.999);
    let usable_bits = settings.target_bytes as f64 * 8.0 * margin;
    let total_kbps = usable_bits / 1000.0 / duration;

    // Audio comes off the top — it's effectively fixed cost.
    let route = audio_route(settings, info, &mut notes);
    let mut audio_kbps = audio_budget(route, settings, info);

    // On very tight targets, a fixed 128k audio track can eat the whole budget.
    // Shrinking audio is far less visible than starving the video.
    let audio_floor = match route {
        AudioCodec::Opus => 32,
        _ => 48,
    };
    if audio_kbps > 0 && (total_kbps - audio_kbps as f64) < MIN_VIDEO_KBPS * 2.0 {
        // A copied track can already sit below the floor — a 32k voice memo —
        // and `clamp` panics when its minimum passes its maximum.
        let squeezed = ((total_kbps * 0.25) as u32).clamp(audio_floor.min(audio_kbps), audio_kbps);
        if squeezed < audio_kbps && route != AudioCodec::Copy {
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
            "{} is too small for a {} video — even at minimum quality it won't fit. Trim it shorter, or pick a larger target.",
            crate::format_size(settings.target_bytes),
            format_duration(duration)
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
    let source_height = source_height.max(1);
    let mut height = settings
        .max_height
        .unwrap_or(source_height)
        .min(source_height);
    let min_bpp = settings.video_codec.min_bpp();

    let bpp_at = |h: u32, fps: f64| -> f64 {
        let w = scaled_width(source_width, source_height, h) as f64;
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
        if height < even_floor(source_height) {
            notes.push(format!(
                "Downscaled to {height}p to keep the picture clean."
            ));
        }
    }

    let height = even_floor(height);
    let width = scaled_width(source_width, source_height, height);

    let encoder = pick_encoder(settings, bins, &mut notes);

    Ok(Plan {
        mode: TargetMode::Size,
        crf: None,
        video_kbps: video_kbps.max(MIN_VIDEO_KBPS).round() as u32,
        audio_kbps,
        width,
        height,
        fps,
        encoder,
        downscaled: height < even_floor(source_height),
        // The target is the estimate in this mode.
        estimated_bytes: None,
        copy_streams: false,
        notes,
    })
}

/// How tall a download is worth fetching, for what it's about to become.
///
/// A ten-minute video squeezed into 10 MB comes out around 360p, but was
/// fetched at 1080p every time — five to ten times the bandwidth, spent on
/// pixels the encode then throws away. When the download goes straight into
/// a size-targeted encode, the height comes from the same bits-per-pixel
/// floor the planner uses, one rung higher for headroom: that estimate
/// assumes 16:9 at 30 fps, and scaling down from a little more detail looks
/// better than from exactly enough.
///
/// Anything that isn't about to be squeezed — quality or keep mode, a link
/// held back for trimming, a length nobody knows — gets the full ceiling.
pub fn download_height(settings: &Settings, duration: Option<f64>) -> u32 {
    let ceiling = settings.max_download_height.max(144);
    let Some(duration) = duration.filter(|d| *d > 0.0) else {
        return ceiling;
    };
    if settings.mode != TargetMode::Size || !settings.auto_compress_downloads {
        return ceiling;
    }

    let margin = settings.safety_margin.clamp(0.5, 0.999);
    let total_kbps = settings.target_bytes as f64 * 8.0 * margin / 1000.0 / duration;
    let audio_kbps = match settings.audio_codec {
        AudioCodec::None => 0.0,
        _ => settings.audio_bitrate_kbps as f64,
    };
    let video_kbps = total_kbps * 0.99 - audio_kbps;
    let carries = |h: u32| {
        let pixels = h as f64 * (h as f64 * 16.0 / 9.0) * 30.0;
        video_kbps * 1000.0 / pixels >= settings.video_codec.min_bpp()
    };

    let lowest = *LADDER.last().unwrap_or(&180);
    let fits = LADDER
        .iter()
        .copied()
        .find(|&h| carries(h))
        .unwrap_or(lowest);
    let with_headroom = LADDER
        .iter()
        .copied()
        .rev()
        .find(|&h| h > fits)
        .unwrap_or(fits);
    with_headroom.max(360).min(ceiling)
}

/// Fills in a copied audio track's bitrate when the file doesn't state one.
///
/// Call between probing and planning. It's a pass over the file, so only
/// done when the audio really is going to be copied.
pub fn measure_copied_audio(
    bins: &Binaries,
    input: &Path,
    info: &mut MediaInfo,
    settings: &Settings,
) {
    let copying = settings.audio_codec == AudioCodec::Copy || settings.mode == TargetMode::Keep;
    if copying && info.has_audio() && info.audio_bitrate_kbps.is_none() {
        info.audio_bitrate_kbps = ffmpeg::measure_audio_kbps(bins, input, info.duration);
    }
}

/// Chooses the encoder, falling back to software when the GPU can't help.
fn pick_encoder(settings: &Settings, bins: &Binaries, notes: &mut Vec<String>) -> String {
    if !settings.hardware {
        return settings.video_codec.software().to_string();
    }

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
            notes.push(format!(
                "Using {hw} — fast, but size accuracy is looser than software."
            ));
            hw.to_string()
        }
        None => {
            notes.push("No usable hardware encoder found; using software.".into());
            settings.video_codec.software().to_string()
        }
    }
}

/// Planning when there's no size to hit — pick a constant quality and keep the
/// picture as it is, apart from any caps the user asked for.
fn plan_by_quality(
    info: &MediaInfo,
    settings: &Settings,
    edits: &Edits,
    bins: &Binaries,
    source_width: u32,
    source_height: u32,
    mut notes: Vec<String>,
) -> Result<Plan, String> {
    let mut fps = info.fps;
    if let Some(cap) = settings.max_fps {
        if fps > cap {
            fps = cap;
            notes.push(format!("Frame rate capped at {} fps.", cap.round()));
        }
    }

    let source_height = source_height.max(1);
    let height = even_floor(
        settings
            .max_height
            .unwrap_or(source_height)
            .min(source_height),
    );
    let width = scaled_width(source_width, source_height, height);

    let encoder = pick_encoder(settings, bins, &mut notes);
    let crf = settings.quality.crf(settings.video_codec);

    notes.push(format!(
        "Encoding at {} quality — the size lands wherever it lands.",
        settings.quality.label()
    ));
    if height < even_floor(source_height) {
        notes.push(format!("Capped at {height}p."));
    }
    notes.push(
        "The estimated size is a guess from the quality setting and frame size; \
         busy footage runs larger and still footage smaller."
            .into(),
    );

    let route = audio_route(settings, info, &mut notes);
    let audio_kbps = audio_budget(route, settings, info);

    let estimated_bytes = estimate_quality_bytes(
        settings.video_codec,
        crf,
        width,
        height,
        fps,
        audio_kbps,
        edits.duration(info.duration),
    );

    Ok(Plan {
        mode: TargetMode::Quality,
        video_kbps: 0,
        crf: Some(crf),
        audio_kbps,
        width,
        height,
        fps,
        encoder,
        downscaled: height < even_floor(source_height),
        estimated_bytes,
        copy_streams: false,
        notes,
    })
}

/// Near-lossless constant quality, for when keep mode has to re-encode anyway.
///
/// Cropping can't be done by copying — the frames genuinely change — so the
/// promise becomes "you won't see the difference" rather than "nothing was
/// touched". These sit a few points below the High preset for that reason.
fn keep_crf(codec: VideoCodec) -> u32 {
    match codec {
        VideoCodec::H264 => 18,
        VideoCodec::H265 => 22,
        VideoCodec::Vp9 => 26,
        VideoCodec::Av1 => 28,
    }
}

/// Applies the edits and leaves everything else alone.
fn plan_by_keeping(
    info: &MediaInfo,
    settings: &Settings,
    edits: &Edits,
    bins: &Binaries,
    source_width: u32,
    source_height: u32,
    mut notes: Vec<String>,
) -> Result<Plan, String> {
    // A crop changes the pixels, so they have to be encoded again. Trimming
    // doesn't: cutting between two points is a copy, whatever the codec.
    let crops = edits.crop.is_some();
    let capped = settings
        .max_height
        .is_some_and(|cap| cap < source_height.max(1))
        || settings.max_fps.is_some_and(|cap| cap < info.fps);
    // Copying keeps the codec, so the container has to be able to hold it —
    // H.264 can't go into WebM however little else changes.
    let fits = video_fits(settings.container, &info.video_codec);
    let copy_streams = !crops && !capped && fits;

    let duration = edits.duration(info.duration);

    if copy_streams {
        notes.push("Copying the video across untouched — no re-encoding.".into());
        if edits.start.is_some() || edits.end.is_some() {
            // Worth saying plainly. Copying means the cut can only land on a
            // keyframe, so asking for 1:03 may give you 1:01.
            notes.push(
                "Cuts land on the nearest keyframe before the point you chose, \
                 which is the price of not re-encoding."
                    .into(),
            );
        }
    } else if crops {
        notes.push("Cropping means re-encoding, so this runs at near-lossless quality.".into());
    } else if !fits {
        notes.push(format!(
            "{} video can't go into {} as it is, so it's re-encoded at near-lossless quality.",
            info.video_codec.to_uppercase(),
            settings.container.extension().to_uppercase()
        ));
    } else {
        notes.push("Re-encoding at near-lossless quality to apply the caps you set.".into());
    }

    let height = even_floor(
        settings
            .max_height
            .unwrap_or(source_height)
            .min(source_height.max(1)),
    );
    let width = scaled_width(source_width, source_height.max(1), height);

    let fps = match settings.max_fps {
        Some(cap) if cap < info.fps => cap,
        _ => info.fps,
    };

    // Audio is copied too, unless the container can't hold it — a camera's PCM
    // track can't go into an MP4 — in which case it's re-encoded and said so.
    let route = keep_audio_route(settings, info, &mut notes);
    let audio_kbps = match route {
        AudioCodec::Copy => info.audio_bitrate_kbps.unwrap_or(0),
        other => audio_budget(other, settings, info),
    };

    // Copying keeps the source's own bitrate, so the size follows from how much
    // of the clip is left. That's arithmetic rather than a guess, unlike the
    // quality-mode estimate.
    let estimated_bytes = if copy_streams && info.duration > 0.0 && info.size_bytes > 0 {
        Some((info.size_bytes as f64 * (duration / info.duration)).round() as u64)
    } else {
        None
    };

    Ok(Plan {
        mode: TargetMode::Keep,
        video_kbps: 0,
        crf: if copy_streams {
            None
        } else {
            Some(keep_crf(settings.video_codec))
        },
        audio_kbps,
        width,
        height,
        fps,
        encoder: if copy_streams {
            "copy".into()
        } else {
            pick_encoder(settings, bins, &mut notes)
        },
        downscaled: height < even_floor(source_height),
        estimated_bytes,
        copy_streams,
        notes,
    })
}

/// Keeps the aspect ratio and forces an even width, which every codec requires.
///
/// Rounded the way ffmpeg's `scale=-2:h` rounds — to the nearest multiple of
/// two — so the plan describes the file that actually comes out.
fn scaled_width(source_width: u32, source_height: u32, height: u32) -> u32 {
    if source_height == 0 || source_width == 0 {
        return 0;
    }
    let ratio = source_width as f64 / source_height as f64;
    ((height as f64 * ratio / 2.0).round() as u32 * 2).max(2)
}

/// Heights are even too. A source odd in both directions — 853x481 — planned
/// its own 481 as the target, and `scale=-2:481` is a height x264 refuses.
fn even_floor(height: u32) -> u32 {
    (height & !1).max(2)
}

/// Whether a container can carry this video codec without re-encoding it.
fn video_fits(container: Container, codec: &str) -> bool {
    match container {
        Container::Mkv => true,
        Container::Webm => matches!(codec, "vp8" | "vp9" | "av1"),
        Container::Mp4 => matches!(codec, "h264" | "hevc" | "av1" | "vp9" | "mpeg4"),
    }
}

/// Whether a container can carry this audio codec as it is.
fn audio_fits(container: Container, codec: &str) -> bool {
    match container {
        Container::Mkv => true,
        Container::Webm => matches!(codec, "opus" | "vorbis"),
        Container::Mp4 => matches!(
            codec,
            "aac" | "mp3" | "ac3" | "eac3" | "opus" | "flac" | "alac"
        ),
    }
}

/// The codec to re-encode into when copying isn't possible.
fn native_audio(container: Container) -> AudioCodec {
    if container == Container::Webm {
        AudioCodec::Opus
    } else {
        AudioCodec::Aac
    }
}

/// What the audio actually becomes.
///
/// "Copy" is a request, not a promise. AAC — the audio in nearly every MP4 —
/// can't go into WebM, and asking ffmpeg to put it there failed the whole
/// encode, so a track the container can't hold is re-encoded and the card
/// says so.
fn audio_route(settings: &Settings, info: &MediaInfo, notes: &mut Vec<String>) -> AudioCodec {
    if !info.has_audio() {
        return AudioCodec::None;
    }
    match settings.audio_codec {
        AudioCodec::Copy => copy_or_convert(settings.container, info, notes),
        other => other,
    }
}

/// Keep mode copies audio whatever the setting says, where it can.
fn keep_audio_route(settings: &Settings, info: &MediaInfo, notes: &mut Vec<String>) -> AudioCodec {
    if !info.has_audio() {
        return AudioCodec::None;
    }
    copy_or_convert(settings.container, info, notes)
}

fn copy_or_convert(container: Container, info: &MediaInfo, notes: &mut Vec<String>) -> AudioCodec {
    let codec = info.audio_codec.as_deref().unwrap_or("");
    if audio_fits(container, codec) {
        return AudioCodec::Copy;
    }
    let instead = native_audio(container);
    notes.push(format!(
        "{} audio can't go into {} as it is, so it's re-encoded as {}.",
        codec.to_uppercase(),
        container.extension().to_uppercase(),
        if instead == AudioCodec::Opus {
            "Opus"
        } else {
            "AAC"
        }
    ));
    instead
}

/// The bitrate the audio will spend, for the budget.
fn audio_budget(route: AudioCodec, settings: &Settings, info: &MediaInfo) -> u32 {
    match route {
        AudioCodec::None => 0,
        AudioCodec::Copy => info
            .audio_bitrate_kbps
            .unwrap_or(settings.audio_bitrate_kbps),
        _ => settings.audio_bitrate_kbps,
    }
}

/// HDR to SDR, for sources that are PQ or HLG.
///
/// Squeezing HDR into 8-bit without mapping it just relabels the numbers: the
/// result is flat and washed out on every ordinary screen, which is all of
/// them as far as a Discord embed is concerned. Linear light, Hable's curve,
/// then back to BT.709.
const TONEMAP: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,\
                       tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p";

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

/// Everything describing *what* to produce, bundled so it can be threaded
/// through planning, argument building and the retry loop as one thing.
pub struct Task<'a> {
    pub input: &'a Path,
    pub info: &'a MediaInfo,
    pub settings: &'a Settings,
    pub edits: &'a Edits,
    /// Preferred output filename, used when the input is a temp download whose
    /// own name means nothing.
    pub name_hint: Option<&'a str>,
}

fn s(v: &str) -> String {
    v.to_string()
}

/// Guards against a preset saved for a different encoder. The UI repairs this
/// when you switch codec, but a settings file written by an older build
/// shouldn't reach ffmpeg as `-preset medium` on an encoder wanting a number.
fn x26x_preset(preset: &str) -> &str {
    const VALID: &[&str] = &[
        "ultrafast",
        "superfast",
        "veryfast",
        "faster",
        "fast",
        "medium",
        "slow",
        "slower",
        "veryslow",
    ];
    if VALID.contains(&preset) {
        preset
    } else {
        "medium"
    }
}

/// Clamps a numeric preset into the range its encoder accepts.
fn numeric_preset(preset: &str, min: u32, max: u32, fallback: u32) -> String {
    preset
        .parse::<u32>()
        .map(|n| n.clamp(min, max))
        .unwrap_or(fallback)
        .to_string()
}

/// Builds the argument list for one ffmpeg pass.
///
/// `pass` is `None` for a single-pass encode, or `Some(1 | 2)` for two-pass.
/// The audio encoder arguments for AAC or Opus.
fn push_audio_encode(args: &mut Vec<String>, codec: AudioCodec, kbps: u32, info: &MediaInfo) {
    if codec == AudioCodec::Opus {
        args.extend([s("-c:a"), s("libopus")]);
        // libopus refuses 5.1(side), the layout most AC3 tracks arrive in, so
        // anything beyond stereo is downmixed, as it already was for AAC.
        if info.audio_channels.unwrap_or(2) > 2 {
            args.extend([s("-ac"), s("2")]);
        }
    } else {
        // Downmix to stereo; surround sources waste bitrate Discord won't play back.
        args.extend([s("-c:a"), s("aac"), s("-ac"), s("2")]);
    }
    args.extend([s("-b:a"), format!("{}k", kbps.max(32))]);
}

fn build_args(
    task: &Task,
    output: &Path,
    plan: &Plan,
    pass: Option<u8>,
    passlog: &Path,
) -> Vec<String> {
    let Task {
        input,
        info,
        settings,
        edits,
        ..
    } = task;

    let mut args: Vec<String> = vec![s("-y")];

    // Seeking before -i is the fast path, and modern ffmpeg still lands on the
    // right frame because it decodes forward from the preceding keyframe.
    let start = edits.start_at(info.duration);
    if start > 0.0 {
        args.push(s("-ss"));
        args.push(format!("{start:.3}"));
    }

    args.push(s("-i"));
    args.push(input.to_string_lossy().into_owned());

    if !edits.is_empty() {
        // Duration rather than an end timestamp, since -ss already shifted the
        // origin and -to would be measured against the original timeline.
        args.push(s("-t"));
        args.push(format!("{:.3}", edits.duration(info.duration)));
    }

    // Keep mode with nothing to re-encode: hand both streams straight through.
    // No filters, no codec settings, no second pass — none of it applies when
    // the packets aren't being decoded in the first place.
    if plan.copy_streams {
        // The picture and the sound, and nothing else. `-map 0` took every
        // stream in the file, and an MKV's subtitles or an iPhone's timecode
        // and metadata tracks have no place in an MP4 — ffmpeg refused the
        // whole copy over them.
        args.extend([s("-map"), s("0:v:0"), s("-map"), s("0:a?")]);
        args.extend([s("-c:v"), s("copy")]);
        match keep_audio_route(settings, info, &mut Vec::new()) {
            AudioCodec::Copy | AudioCodec::None => args.extend([s("-c:a"), s("copy")]),
            other => push_audio_encode(&mut args, other, plan.audio_kbps, info),
        }
        args.extend([s("-sn"), s("-dn")]);
        // Timestamps start wherever the keyframe was, and some players show a
        // clip that begins at 0:47 as being 47 seconds long with a blank start.
        args.extend([s("-avoid_negative_ts"), s("make_zero")]);
        if matches!(task.settings.container, Container::Mp4) {
            args.extend([s("-movflags"), s("+faststart")]);
        }
        args.push(output.to_string_lossy().into_owned());
        return args;
    }

    // Crop first: everything after it works on the smaller frame.
    let (cropped_width, cropped_height) = edits.dimensions(info.width, info.height);
    let mut filters: Vec<String> = Vec::new();
    if let Some(crop) = edits.crop_filter(info.width, info.height) {
        filters.push(crop);
    }

    // A still is one frame written back as a picture, not a video encode.
    //
    // Everything below assumes a timeline — a codec, a bitrate or a CRF, an
    // audio stream — and none of it applies. Running a photo through it is why
    // cropping one appeared to do nothing: the crop was correct, but the result
    // was an attempt at a video of a single frame rather than the image back.
    if still_extension(input).is_some() {
        let mut args = vec![s("-y"), s("-i"), input.to_string_lossy().into_owned()];
        if !filters.is_empty() {
            args.extend([s("-vf"), filters.join(",")]);
        }
        args.extend([s("-frames:v"), s("1")]);
        // Each image encoder reads quality its own way. `-q:v 2` is near the
        // top for JPEG, but libwebp takes it as 2 out of 100 — which is how a
        // cropped WebP came back a third the size and visibly smeared.
        match still_extension(input) {
            Some("jpg") => args.extend([s("-q:v"), s("2")]),
            Some("webp") => args.extend([s("-quality"), s("90")]),
            _ => {}
        }
        args.push(output.to_string_lossy().into_owned());
        return args;
    }
    if plan.height != cropped_height || plan.width != cropped_width {
        filters.push(format!("scale=-2:{}:flags=lanczos", plan.height));
    }
    if (plan.fps - info.fps).abs() > 0.01 {
        filters.push(format!("fps={:.3}", plan.fps));
    }
    // After scaling, so the expensive part runs on the smaller frame.
    if info.is_hdr() {
        filters.push(TONEMAP.split_whitespace().collect());
    }

    // A GIF that reaches here was edited, and it comes back a GIF.
    //
    // None of the video machinery below applies: there's no codec to pick, no
    // bitrate to spend and no audio. Size is bought with pixels and frames
    // instead, which is what the retry loop adjusts when one lands over target.
    if is_gif(input) {
        let chain = if filters.is_empty() {
            String::new()
        } else {
            format!("{},", filters.join(","))
        };
        args.extend([
            s("-filter_complex"),
            // Same two-pass palette as the converter: one pass works out the
            // best colours for this clip, the second maps every frame onto
            // them. Letting the encoder pick a fixed palette is what makes
            // most GIFs look like they escaped from 1998.
            format!(
                "{chain}split[a][b];\
                 [a]palettegen=max_colors=128:stats_mode=diff[p];\
                 [b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
            ),
            s("-an"),
            s("-loop"),
            s("0"),
        ]);
        args.push(output.to_string_lossy().into_owned());
        return args;
    }

    if !filters.is_empty() {
        args.push(s("-vf"));
        args.push(filters.join(","));
    }

    args.push(s("-c:v"));
    args.push(plan.encoder.clone());

    let is_hardware_encoder = !plan.encoder.starts_with("lib");
    match plan.crf {
        // Constant quality: the encoder decides the bitrate frame by frame.
        Some(crf) => {
            if is_hardware_encoder {
                // Each vendor spells constant quality differently.
                if plan.encoder.contains("nvenc") {
                    args.extend([s("-cq"), crf.to_string()]);
                    args.extend([s("-rc"), s("vbr"), s("-b:v"), s("0")]);
                } else if plan.encoder.contains("qsv") {
                    args.extend([s("-global_quality"), crf.to_string()]);
                } else if plan.encoder.contains("amf") {
                    // AMF has no `-qp`: ffmpeg only warned that it went
                    // unused, and the encode quietly ran at the encoder's
                    // default rate control instead. It takes one per frame
                    // type, and HEVC has no B-frames to set.
                    args.extend([s("-rc"), s("cqp")]);
                    args.extend([s("-qp_i"), crf.to_string(), s("-qp_p"), crf.to_string()]);
                    if !plan.encoder.contains("hevc") {
                        args.extend([s("-qp_b"), crf.to_string()]);
                    }
                } else {
                    args.extend([s("-qp"), crf.to_string()]);
                }
            } else {
                args.extend([s("-crf"), crf.to_string()]);
                // VP9 needs an explicit zero bitrate or -crf is ignored.
                if plan.encoder == "libvpx-vp9" {
                    args.extend([s("-b:v"), s("0")]);
                }
            }
        }
        None => {
            args.push(s("-b:v"));
            args.push(format!("{}k", plan.video_kbps));
        }
    }

    // Encoder-specific knobs. Every encoder spells "how hard should I work"
    // differently, so the UI's preset is translated here rather than there.
    let is_hardware = !plan.encoder.starts_with("lib");
    let preset = settings.preset.as_str();

    match plan.encoder.as_str() {
        "libx264" | "libx265" => {
            args.extend([s("-preset"), s(x26x_preset(preset))]);
        }
        "libvpx-vp9" => {
            // Row threading is not optional — VP9 is glacial without it.
            // Note cpu-used runs backwards: higher means less effort.
            args.extend([
                s("-row-mt"),
                s("1"),
                s("-deadline"),
                s("good"),
                s("-cpu-used"),
                numeric_preset(preset, 0, 5, 2),
            ]);
        }
        "libsvtav1" => {
            args.extend([s("-preset"), numeric_preset(preset, 0, 13, 6)]);
        }
        e if e.contains("nvenc") => {
            args.extend([
                s("-preset"),
                s(match preset {
                    "quality" => "p7",
                    "speed" => "p1",
                    _ => "p4",
                }),
                s("-rc"),
                s("vbr"),
            ]);
        }
        e if e.contains("qsv") => {
            args.extend([
                s("-preset"),
                s(match preset {
                    "quality" => "veryslow",
                    "speed" => "veryfast",
                    _ => "medium",
                }),
            ]);
        }
        e if e.contains("amf") => {
            args.extend([
                s("-quality"),
                s(match preset {
                    "quality" => "quality",
                    "speed" => "speed",
                    _ => "balanced",
                }),
            ]);
        }
        // videotoolbox exposes no comparable effort control.
        _ => {}
    }

    // Only meaningful when aiming at a size: hardware rate control drifts, so
    // it gets an explicit ceiling. In quality mode there's nothing to clamp.
    if is_hardware && plan.crf.is_none() {
        args.extend([
            s("-maxrate"),
            format!("{}k", (plan.video_kbps as f64 * 1.35) as u32),
            s("-bufsize"),
            format!("{}k", plan.video_kbps * 2),
        ]);
    }

    // H.265 in MP4 needs the hvc1 tag or most players refuse it.
    if settings.container == Container::Mp4
        && (plan.encoder.contains("265") || plan.encoder.contains("hevc"))
    {
        args.extend([s("-tag:v"), s("hvc1")]);
    }

    // Broad-compatibility pixel format — some sources are 10-bit or 4:2:2,
    // which many players (and Discord's inline preview) won't touch. Every
    // encoder, hardware ones included: NVENC accepts 4:4:4 and 10-bit input and
    // passes it straight through, so a screen capture came out as H.264 High
    // 4:4:4, which no browser plays. The GPU encoders all take NV12.
    args.extend([
        s("-pix_fmt"),
        s(if is_hardware { "nv12" } else { "yuv420p" }),
    ]);
    if info.is_hdr() {
        // The tone map made it BT.709; say so, rather than letting the
        // source's BT.2020 tags ride along and mislabel it.
        args.extend([
            s("-color_primaries"),
            s("bt709"),
            s("-color_trc"),
            s("bt709"),
            s("-colorspace"),
            s("bt709"),
        ]);
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
    match audio_route(settings, info, &mut Vec::new()) {
        AudioCodec::None => args.push(s("-an")),
        AudioCodec::Copy => args.extend([s("-c:a"), s("copy")]),
        other => push_audio_encode(&mut args, other, plan.audio_kbps, info),
    }
    // Subtitles and data tracks are dropped. Discord shows neither, and a
    // bitmap subtitle can't be converted into MP4's text format at all.
    args.extend([s("-sn"), s("-dn")]);

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
    task: &Task,
    cancel: &Arc<AtomicBool>,
    on_progress: impl FnMut(f64, &str),
) -> Result<Result<EncodeOutcome, Cancelled>, String> {
    let plan = plan(task.info, task.settings, task.edits, bins)?;
    run_planned(bins, task, plan, cancel, on_progress)
}

/// Runs an encode to a plan already made.
///
/// Split from `run_job` so a test can hand it a plan that names a hardware
/// encoder on a machine without one, which is the only way to watch the
/// fallback below happen.
pub fn run_planned(
    bins: &Binaries,
    task: &Task,
    mut plan: Plan,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(f64, &str),
) -> Result<Result<EncodeOutcome, Cancelled>, String> {
    let Task {
        input,
        info,
        settings,
        edits,
        name_hint,
    } = task;

    // Everything below writes to `dest.partial`. Any way out of here other than
    // the `commit` at the end — an error, a cancel, a miss on the target —
    // drops `dest`, which takes both the partial file and the claimed name
    // back out of the user's folder.
    let dest = output_destination(input, settings, *name_hint)?;
    let output = dest.partial.clone();

    // Progress is measured against what actually gets written, which after a
    // trim is shorter than the source.
    let encoded_duration = edits.duration(info.duration);

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

    // Two passes exist to hit a size accurately. Constant quality has no size
    // to hit, so the analysis pass would be pure waste — and a stream copy has
    // no encoder to analyse anything with.
    let wants_two_pass = |plan: &Plan| {
        plan.crf.is_none()
            && !plan.copy_streams
            && settings.two_pass
            && settings.video_codec.supports_two_pass()
            && plan.encoder.starts_with("lib")
            // A GIF isn't encoded by any of that, and an analysis pass over one
            // writes a log for an encoder that never runs.
            && !is_gif(task.input)
    };
    let mut two_pass = wants_two_pass(&plan);

    let mut attempts = 0;
    let mut final_bytes;

    loop {
        attempts += 1;

        if two_pass {
            let args = build_args(task, &output, &plan, Some(1), &passlog);
            let stage = if attempts > 1 {
                "Analysing (retry)"
            } else {
                "Analysing"
            };
            match ffmpeg::run(bins, &args, encoded_duration, cancel, |p| {
                on_progress(p * 0.35, stage)
            })? {
                Ok(()) => {}
                Err(c) => {
                    cleanup_passlog(&passlog);
                    return Ok(Err(c));
                }
            }

            let args = build_args(task, &output, &plan, Some(2), &passlog);
            match ffmpeg::run(bins, &args, encoded_duration, cancel, |p| {
                on_progress(0.35 + p * 0.65, "Encoding")
            })? {
                Ok(()) => {}
                Err(c) => {
                    cleanup_passlog(&passlog);
                    return Ok(Err(c));
                }
            }
        } else {
            let args = build_args(task, &output, &plan, None, &passlog);
            match ffmpeg::run(bins, &args, encoded_duration, cancel, |p| {
                on_progress(p, "Encoding")
            }) {
                Ok(Ok(())) => {}
                Ok(Err(c)) => {
                    cleanup_passlog(&passlog);
                    return Ok(Err(c));
                }
                // A GPU encoder that passed its two-frame test can still give
                // up on the real thing — a driver reset, a resolution or pixel
                // format the chip won't take, another app holding the encoder.
                // That used to fail the job outright; it's finished in
                // software instead, which is slower but always works.
                Err(error) if !plan.encoder.starts_with("lib") => {
                    let failed = std::mem::replace(
                        &mut plan.encoder,
                        settings.video_codec.software().to_string(),
                    );
                    plan.notes.push(format!(
                        "{failed} stopped partway, so this was finished in software."
                    ));
                    eprintln!("[encode] {failed} failed, falling back to software: {error}");
                    two_pass = wants_two_pass(&plan);
                    attempts -= 1;
                    on_progress(0.0, "Retrying in software");
                    continue;
                }
                Err(error) => {
                    cleanup_passlog(&passlog);
                    return Err(error);
                }
            }
        }

        final_bytes = std::fs::metadata(&output)
            .map_err(|e| format!("Couldn't read the finished file: {e}"))?
            .len();

        // Rate control aims, it doesn't promise. If we overshot, scale the
        // bitrate by how far off we were and go again. Quality and keep modes
        // have no ceiling to overshoot, so whatever came out is the answer.
        if plan.crf.is_some()
            || plan.copy_streams
            || final_bytes <= settings.target_bytes
            || attempts >= MAX_ATTEMPTS
        {
            break;
        }

        let correction = (settings.target_bytes as f64 / final_bytes as f64) * 0.97;

        // A GIF has no bitrate to turn down. What it costs is pixels and
        // frames, so that's what gets reduced — by the square root of the
        // correction, since the frame is two-dimensional and halving each side
        // quarters the area.
        if is_gif(task.input) {
            let shrink = correction.sqrt().clamp(0.35, 0.95);
            let reduced = ((plan.height as f64 * shrink) as u32).max(MIN_GIF_HEIGHT);
            plan.height = reduced & !1;
            plan.fps = (plan.fps * shrink).max(MIN_GIF_FPS);
            plan.notes.push(format!(
                "Overshot; retrying at {}p and {:.0} fps.",
                plan.height, plan.fps
            ));
        } else {
            let corrected = (plan.video_kbps as f64 * correction).max(MIN_VIDEO_KBPS);
            plan.video_kbps = corrected.round() as u32;
            plan.notes
                .push(format!("Overshot; retrying at {} kbps.", plan.video_kbps));
        }
        on_progress(0.0, "Retrying");
    }

    cleanup_passlog(&passlog);

    // Only a failure when a size was actually being aimed at. In quality and
    // keep modes the target is meaningless, and judging the result against it
    // would fail almost every encode for doing exactly what was asked.
    if plan.crf.is_none() && !plan.copy_streams && final_bytes > settings.target_bytes {
        return Err(format!(
            "Couldn't get under {} after {attempts} attempts (landed at {}). Try a lower resolution or a newer codec.",
            crate::format_size(settings.target_bytes),
            crate::format_size(final_bytes)
        ));
    }

    let output = dest.commit()?;
    Ok(Ok(EncodeOutcome {
        output,
        final_bytes,
        attempts,
        plan,
    }))
}

/// Whether this file can simply be handed over untouched.
///
/// Re-encoding something that already fits only throws quality away, so the
/// honest answer to "make this 10 MB" for an 8 MB clip is to do nothing. The
/// container still has to be one Discord previews inline, otherwise "it fits"
/// would be technically true and practically useless.
/// The extension of a still, or `None` for anything with a timeline.
///
/// A GIF is deliberately absent: it has frames, so it goes down the video path
/// like everything else that moves.
pub fn still_extension(input: &Path) -> Option<&'static str> {
    let ext = input.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        _ => None,
    }
}

/// A GIF, which is kept as one rather than turned into video.
pub fn is_gif(input: &Path) -> bool {
    input
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

pub fn can_pass_through(
    info: &MediaInfo,
    settings: &Settings,
    edits: &Edits,
    input: &Path,
) -> bool {
    if !edits.is_empty() || edits.force_encode {
        return false;
    }

    // A GIF is handed over untouched, whatever size it is and whatever target
    // is set. Squeezing one into a size limit means 256 colours and a visible
    // mess, and the answer that actually works — make it video — isn't what
    // was asked for by dropping in a GIF. Opening the editor is the deliberate
    // act that unlocks re-encoding one, and it still comes back a GIF.
    if is_gif(input) {
        return true;
    }

    // Quality mode is an explicit "re-encode this", so there's nothing to skip.
    if settings.mode == TargetMode::Quality {
        return false;
    }
    // Keep mode with no edits asks for nothing at all to happen, whatever the
    // size or container — there's no target to measure against and no filter to
    // apply, so copying the file is the complete job.
    if settings.mode == TargetMode::Keep {
        return true;
    }
    if info.size_bytes == 0 || info.size_bytes > settings.target_bytes {
        return false;
    }

    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Stills are handed over as they are whenever they fit, which is nearly
    // always — a photo from a post is a few hundred kilobytes against a ten
    // megabyte limit. Re-encoding one to "save" space it isn't using would
    // cost quality for nothing, and the bitrate arithmetic below has no
    // meaning for something with no duration.
    if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp") {
        return true;
    }

    // A video only fits if Discord will actually play it where it lands. The
    // extension alone let an iPhone's HEVC `.mov` through untouched — under
    // the limit, and a download link rather than a video on most clients —
    // along with AV1, 10-bit and HDR files that fail the same way.
    plays_inline(info, &ext)
}

/// Whether Discord's clients play this file in the chat rather than offering
/// it as a download: H.264 or VP8/VP9 in their usual containers, 8-bit 4:2:0,
/// SDR, with audio a browser decodes.
fn plays_inline(info: &MediaInfo, ext: &str) -> bool {
    let audio = info.audio_codec.as_deref();
    let streams_fit = match ext {
        "mp4" | "m4v" | "mov" => {
            info.video_codec == "h264" && matches!(audio, None | Some("aac" | "mp3"))
        }
        "webm" => {
            matches!(info.video_codec.as_str(), "vp8" | "vp9")
                && matches!(audio, None | Some("opus" | "vorbis"))
        }
        _ => false,
    };
    streams_fit && info.is_plain_8bit() && !info.is_hdr()
}

/// Strips anything Windows or the user would object to in a filename.
///
/// Video titles arrive from the internet, so this has to cope with path
/// separators, control characters, and names that are simply enormous.
pub fn safe_stem(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();

    let clipped: String = cleaned
        .trim()
        .trim_matches('.')
        .trim()
        .chars()
        .take(80)
        .collect();
    let result = clipped.trim().to_string();

    if result.is_empty() {
        return "video".into();
    }

    // Windows reserves these as device names whatever follows the dot, so a
    // video titled "CON" can't become `CON.mp4` — creating it fails, or worse,
    // opens the console.
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED
        .iter()
        .any(|name| result.eq_ignore_ascii_case(name))
    {
        return format!("{result}_");
    }
    result
}

/// Where finished files land.
pub fn output_dir(settings: &Settings) -> Result<PathBuf, String> {
    let dir = match &settings.output_dir {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => dirs_downloads().ok_or("Couldn't find your Downloads folder.")?,
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create the output folder: {e}"))?;
    Ok(dir)
}

/// Moves a file that needs no re-encoding into the output folder, keeping the
/// extension it already has rather than the one the settings ask for.
pub fn place_untouched(
    source: &Path,
    settings: &Settings,
    name_hint: Option<&str>,
) -> Result<PathBuf, String> {
    let dir = output_dir(settings)?;
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_lowercase();

    let stem = match name_hint {
        Some(hint) => safe_stem(hint),
        None => source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video".into()),
    };

    let dest = Destination::claim(&dir, &stem, &ext)?;

    // Rename first — it's instant when both sides are on the same volume, and
    // replaces the placeholder in one step. A temp download usually isn't on
    // the same volume, so the copy goes to the partial name first: a copy
    // interrupted halfway must not leave a file that looks finished.
    if std::fs::rename(source, &dest.path).is_ok() {
        return Ok(dest.keep());
    }
    std::fs::copy(source, &dest.partial)
        .map_err(|e| format!("Couldn't move the file into place: {e}"))?;
    let placed = dest.commit()?;
    let _ = std::fs::remove_file(source);
    Ok(placed)
}

/// Picks a destination that won't clobber anything the user already has.
fn output_destination(
    input: &Path,
    settings: &Settings,
    name_hint: Option<&str>,
) -> Result<Destination, String> {
    let dir = output_dir(settings)?;

    let stem = match name_hint {
        // Downloads land in a temp folder under a fixed name, so the title from
        // the site is the only meaningful thing to call the result.
        Some(hint) => safe_stem(hint),
        None => input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video".into()),
    };
    // A still keeps the format it arrived in. Writing a cropped photo into an
    // MP4 container would be nonsense, and the container setting is about
    // video — it has nothing to say about a JPEG. A GIF is the same promise:
    // trimming one gives back a GIF, not the video it would compress better as.
    let ext = still_extension(input)
        .or_else(|| is_gif(input).then_some("gif"))
        .unwrap_or_else(|| settings.container.extension());

    Destination::claim(&dir, &format!("{stem}-nitrate"), ext)
}

/// A name claimed in the output folder, and the file written before it.
///
/// Checking that a name was free and then letting ffmpeg create it much later
/// — pass 2 doesn't start writing until pass 1 is done — left room for another
/// job to pick the same name in between: two `clip.mp4`s from different
/// folders, or two links with the same title, both became `clip-nitrate.mp4`
/// and one overwrote the other. The name is now claimed by creating it, which
/// only one caller can do.
///
/// The work goes into a `.partial` file beside it and is renamed over the
/// placeholder once it's whole. A cancel or a failure used to leave the
/// half-written file behind under the finished name — an MP4 with no index,
/// which won't play, sitting in Downloads looking done. Dropped without
/// `commit`, this removes both.
struct Destination {
    path: PathBuf,
    partial: PathBuf,
    done: bool,
}

impl Destination {
    fn claim(dir: &Path, stem: &str, ext: &str) -> Result<Destination, String> {
        for n in 1..1000 {
            let name = if n == 1 {
                stem.to_string()
            } else {
                format!("{stem}-{n}")
            };
            let path = dir.join(format!("{name}.{ext}"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => {
                    // Still ending in the real extension, which is how ffmpeg
                    // picks the container to write.
                    let partial = dir.join(format!("{name}.partial.{ext}"));
                    return Ok(Destination {
                        path,
                        partial,
                        done: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("Couldn't write to {}: {e}", dir.display())),
            }
        }
        Err("Too many files with that name already.".into())
    }

    /// Moves the finished partial file into the claimed name.
    fn commit(self) -> Result<PathBuf, String> {
        std::fs::rename(&self.partial, &self.path)
            .map_err(|e| format!("Couldn't put the finished file in place: {e}"))?;
        Ok(self.keep())
    }

    /// The claimed name now holds the real file, however it got there.
    fn keep(mut self) -> PathBuf {
        self.done = true;
        std::mem::take(&mut self.path)
    }
}

impl Drop for Destination {
    fn drop(&mut self) {
        if !self.done {
            let _ = std::fs::remove_file(&self.partial);
            let _ = std::fs::remove_file(&self.path);
        }
    }
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
    // With the separator, or job 1's cleanup matches `-10-0.log` through
    // `-19-0.log` too and deletes stats another job's second pass still needs.
    let prefix = format!("{prefix}-");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> MediaInfo {
        MediaInfo {
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            video_codec: "h264".into(),
            audio_codec: Some("aac".into()),
            audio_bitrate_kbps: Some(128),
            size_bytes: 10_000_000,
            pix_fmt: Some("yuv420p".into()),
            color_transfer: None,
            audio_channels: Some(2),
        }
    }

    fn plan_with(encoder: &str, crf: Option<u32>) -> Plan {
        Plan {
            mode: TargetMode::Quality,
            crf,
            video_kbps: 2000,
            audio_kbps: 128,
            width: 1280,
            height: 720,
            fps: 30.0,
            encoder: encoder.into(),
            downscaled: false,
            estimated_bytes: None,
            copy_streams: false,
            notes: Vec::new(),
        }
    }

    fn args_for(input: &str, plan: &Plan, edits: Edits) -> Vec<String> {
        let info = info();
        let settings = Settings::default();
        let task = Task {
            input: Path::new(input),
            info: &info,
            settings: &settings,
            edits: &edits,
            name_hint: None,
        };
        build_args(&task, Path::new("out"), plan, None, Path::new("log"))
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn amd_constant_quality_uses_the_flags_amf_has() {
        let args = args_for("in.mp4", &plan_with("h264_amf", Some(23)), Edits::default());
        assert!(has_pair(&args, "-rc", "cqp"), "{args:?}");
        assert!(has_pair(&args, "-qp_i", "23") && has_pair(&args, "-qp_p", "23"));
        assert!(has_pair(&args, "-qp_b", "23"));
        assert!(!args.iter().any(|a| a == "-qp"), "AMF has no -qp: {args:?}");

        // HEVC on AMF has no B-frames to set.
        let hevc = args_for("in.mp4", &plan_with("hevc_amf", Some(27)), Edits::default());
        assert!(has_pair(&hevc, "-qp_i", "27") && !hevc.iter().any(|a| a == "-qp_b"));
    }

    #[test]
    fn gpu_encodes_get_a_pixel_format_browsers_play() {
        let args = args_for("in.mp4", &plan_with("h264_nvenc", None), Edits::default());
        assert!(has_pair(&args, "-pix_fmt", "nv12"), "{args:?}");
    }

    #[test]
    fn a_cropped_webp_keeps_its_quality() {
        let crop = Edits {
            crop: Some(CropRect {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            }),
            ..Edits::default()
        };
        let webp = args_for("photo.webp", &plan_with("libx264", None), crop);
        assert!(has_pair(&webp, "-quality", "90"), "{webp:?}");
        assert!(
            !webp.iter().any(|a| a == "-q:v"),
            "2/100 to libwebp: {webp:?}"
        );

        let jpg = args_for("photo.jpg", &plan_with("libx264", None), crop);
        assert!(has_pair(&jpg, "-q:v", "2"));
    }

    #[test]
    fn passlog_cleanup_leaves_other_jobs_alone() {
        let dir = std::env::temp_dir().join(format!("nitrate-passlog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = ["job-1-0.log", "job-1-0.log.mbtree"];
        let theirs = ["job-10-0.log", "job-12-0.log.mbtree", "job-100-0.log"];
        for name in mine.iter().chain(&theirs) {
            std::fs::write(dir.join(name), b"stats").unwrap();
        }

        cleanup_passlog(&dir.join("job-1"));

        for name in mine {
            assert!(!dir.join(name).exists(), "{name} should be gone");
        }
        for name in theirs {
            assert!(dir.join(name).exists(), "{name} belongs to another job");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
