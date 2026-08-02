//! Locating, probing and driving the bundled ffmpeg/ffprobe binaries.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Keeps ffmpeg from flashing a console window on every invocation.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// ffmpeg writes to the platform's bit bucket during the analysis pass.
#[cfg(windows)]
pub const NULL_SINK: &str = "NUL";
#[cfg(not(windows))]
pub const NULL_SINK: &str = "/dev/null";

#[derive(Debug, Clone)]
pub struct Binaries {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// Resolves ffmpeg/ffprobe, in order of preference:
///
/// 1. Alongside our own executable — this is where Tauri drops bundled sidecars.
/// 2. `src-tauri/binaries/<name>-<target-triple>` — where they live during `tauri dev`.
/// 3. Bare name, letting the OS search `PATH` — the developer fallback.
pub fn resolve() -> Binaries {
    let mut ffmpeg = PathBuf::from(exe_name("ffmpeg"));
    let mut ffprobe = PathBuf::from(exe_name("ffprobe"));

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled_ffmpeg = dir.join(exe_name("ffmpeg"));
            let bundled_ffprobe = dir.join(exe_name("ffprobe"));
            if bundled_ffmpeg.is_file() {
                ffmpeg = bundled_ffmpeg;
            }
            if bundled_ffprobe.is_file() {
                ffprobe = bundled_ffprobe;
            }
        }
    }

    // During `tauri dev` the sidecars still carry their target-triple suffix.
    let triple = current_triple();
    let dev_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    let dev_ffmpeg = dev_dir.join(exe_name(&format!("ffmpeg-{triple}")));
    let dev_ffprobe = dev_dir.join(exe_name(&format!("ffprobe-{triple}")));
    if !ffmpeg.is_absolute() && dev_ffmpeg.is_file() {
        ffmpeg = dev_ffmpeg;
    }
    if !ffprobe.is_absolute() && dev_ffprobe.is_file() {
        ffprobe = dev_ffprobe;
    }

    Binaries { ffmpeg, ffprobe }
}

fn current_triple() -> &'static str {
    // Matches the suffix Tauri expects on `externalBin` entries.
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn base_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

// ---------------------------------------------------------------------------
// Probing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    /// Needed to budget correctly when the audio track is stream-copied.
    pub audio_bitrate_kbps: Option<u32>,
    pub size_bytes: u64,
}

impl MediaInfo {
    pub fn has_audio(&self) -> bool {
        self.audio_codec.is_some()
    }
}

#[derive(Deserialize)]
struct ProbeOutput {
    streams: Vec<ProbeStream>,
    format: ProbeFormat,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    size: Option<String>,
}

/// Parses ffprobe's `num/den` rational frame-rate notation.
fn parse_rational(value: &str) -> Option<f64> {
    let (num, den) = value.split_once('/')?;
    let num: f64 = num.trim().parse().ok()?;
    let den: f64 = den.trim().parse().ok()?;
    if den == 0.0 || num == 0.0 {
        return None;
    }
    Some(num / den)
}

pub fn probe(bins: &Binaries, input: &Path) -> Result<MediaInfo, String> {
    let output = base_command(&bins.ffprobe)
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(input)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't run ffprobe: {e}"))?;

    if !output.status.success() {
        return Err("ffprobe couldn't read this file — it may not be a video.".into());
    }

    let parsed: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Couldn't parse ffprobe output: {e}"))?;

    let video = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .ok_or("This file has no video stream.")?;

    let audio = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));

    // Container duration is the most reliable; fall back to the video stream's own.
    let duration = parsed
        .format
        .duration
        .as_deref()
        .and_then(|d| d.parse::<f64>().ok())
        .or_else(|| {
            video
                .duration
                .as_deref()
                .and_then(|d| d.parse::<f64>().ok())
        })
        .filter(|d| *d > 0.0)
        .ok_or("Couldn't determine how long this video is.")?;

    let fps = video
        .avg_frame_rate
        .as_deref()
        .and_then(parse_rational)
        .or_else(|| video.r_frame_rate.as_deref().and_then(parse_rational))
        .unwrap_or(30.0)
        .clamp(1.0, 240.0);

    let size_bytes = parsed
        .format
        .size
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| std::fs::metadata(input).ok().map(|m| m.len()))
        .unwrap_or(0);

    Ok(MediaInfo {
        duration,
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        fps,
        video_codec: video.codec_name.clone().unwrap_or_else(|| "unknown".into()),
        audio_codec: audio.and_then(|a| a.codec_name.clone()),
        audio_bitrate_kbps: audio
            .and_then(|a| a.bit_rate.as_deref())
            .and_then(|b| b.parse::<u64>().ok())
            .map(|bps| (bps / 1000).max(1) as u32),
        size_bytes,
    })
}

/// Grabs a single frame partway through the video.
///
/// `width` matters: a card thumbnail wants a small one, but the editor's
/// preview is shown at several hundred pixels and looks soft if it's upscaled
/// from a postage stamp.
pub fn thumbnail(
    bins: &Binaries,
    input: &Path,
    out: &Path,
    at: f64,
    width: u32,
) -> Result<(), String> {
    let status = base_command(&bins.ffmpeg)
        .args(["-y", "-ss"])
        .arg(format!("{at:.3}"))
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("scale={width}:-2:flags=bilinear"))
        .args(["-q:v", "4"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Couldn't run ffmpeg: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("Thumbnail extraction failed.".into())
    }
}

// ---------------------------------------------------------------------------
// Running an encode with live progress
// ---------------------------------------------------------------------------

pub struct Cancelled;

/// Runs ffmpeg, feeding fractional progress (0.0–1.0) to `on_progress`.
///
/// `-progress pipe:1` gives us a stable key=value stream on stdout, which is far
/// more robust to parse than scraping the human-readable stderr stats line.
pub fn run(
    bins: &Binaries,
    args: &[String],
    duration: f64,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(f64),
) -> Result<Result<(), Cancelled>, String> {
    let mut child = base_command(&bins.ffmpeg)
        .args([
            "-hide_banner",
            "-nostdin",
            "-nostats",
            "-progress",
            "pipe:1",
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start ffmpeg: {e}"))?;

    // ffmpeg will block once the stderr pipe fills, so it must be drained
    // continuously. We keep a rolling tail to explain failures.
    let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
    let stderr_handle = {
        let tail = Arc::clone(&stderr_tail);
        let stderr = child.stderr.take();
        std::thread::spawn(move || {
            let Some(stderr) = stderr else { return };
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let mut tail = tail.lock().unwrap();
                tail.push(line);
                if tail.len() > 40 {
                    tail.remove(0);
                }
            }
        })
    };

    let mut was_cancelled = false;
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                was_cancelled = true;
                break;
            }
            if let Some(secs) = parse_progress_time(&line) {
                if duration > 0.0 {
                    on_progress((secs / duration).clamp(0.0, 1.0));
                }
            }
        }
    }

    let status = child
        .wait()
        .map_err(|e| format!("ffmpeg didn't exit cleanly: {e}"))?;
    let _ = stderr_handle.join();

    if was_cancelled || cancel.load(Ordering::Relaxed) {
        return Ok(Err(Cancelled));
    }

    if status.success() {
        on_progress(1.0);
        Ok(Ok(()))
    } else {
        let tail = stderr_tail.lock().unwrap();
        Err(summarise_failure(&tail))
    }
}

/// Extracts elapsed output time from a `-progress` line.
///
/// `out_time` is preferred because it's unambiguous; ffmpeg's `out_time_ms` has
/// long actually carried *microseconds*, so it's only used as a last resort.
fn parse_progress_time(line: &str) -> Option<f64> {
    let (key, value) = line.split_once('=')?;
    match key.trim() {
        "out_time" => parse_timecode(value.trim()),
        "out_time_us" | "out_time_ms" => {
            let micros: f64 = value.trim().parse().ok()?;
            Some(micros / 1_000_000.0)
        }
        _ => None,
    }
}

fn parse_timecode(value: &str) -> Option<f64> {
    // HH:MM:SS.microseconds
    let mut parts = value.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

/// Turns ffmpeg's stderr tail into something a human can act on.
fn summarise_failure(tail: &[String]) -> String {
    let interesting = tail.iter().rev().find(|line| {
        let l = line.to_lowercase();
        l.contains("error")
            || l.contains("invalid")
            || l.contains("no such file")
            || l.contains("unknown encoder")
            || l.contains("not supported")
            || l.contains("permission denied")
    });

    match interesting {
        Some(line) => {
            let line = line.trim();
            if line.to_lowercase().contains("unknown encoder") {
                format!("{line} — try a different codec in Advanced settings.")
            } else {
                line.to_string()
            }
        }
        None => tail
            .last()
            .cloned()
            .unwrap_or_else(|| "ffmpeg failed without explanation.".into()),
    }
}

/// Asks ffmpeg which encoders this build actually has, so the UI can grey out
/// hardware options the machine can't use.
pub fn available_encoders(bins: &Binaries) -> Vec<String> {
    let Ok(output) = base_command(&bins.ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .output()
    else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            // Lines look like: " V....D h264_nvenc  NVIDIA NVENC H.264 encoder"
            let trimmed = line.trim_start();
            let flags = trimmed.split_whitespace().next()?;
            if flags.len() < 6 || !flags.starts_with('V') {
                return None;
            }
            trimmed.split_whitespace().nth(1).map(str::to_string)
        })
        .collect()
}

/// A hardware encoder can be present in the build but still fail at runtime when
/// the GPU isn't there, so we ask it to encode two throwaway frames.
pub fn encoder_works(bins: &Binaries, encoder: &str) -> bool {
    base_command(&bins.ffmpeg)
        .args([
            "-hide_banner",
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=256x256:d=0.1",
            "-c:v",
            encoder,
            "-frames:v",
            "2",
            "-f",
            "null",
            NULL_SINK,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Best-effort cleanup of a child that outlived its usefulness.
#[allow(dead_code)]
pub fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Drains a reader without caring about its contents.
#[allow(dead_code)]
fn drain(mut r: impl Read) {
    let mut buf = Vec::new();
    let _ = r.read_to_end(&mut buf);
}
